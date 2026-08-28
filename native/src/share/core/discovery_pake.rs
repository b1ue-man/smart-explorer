use std::fmt;

use argon2::{Algorithm, Argon2, Block, Params, Version};
use chacha20::{
    rand_core::{Rng as InfallibleRng, SeedableRng as ChaChaSeedableRng},
    ChaCha20Rng,
};
use opaque_ke::ciphersuite::CipherSuite;
use opaque_ke::errors::InternalError;
use opaque_ke::generic_array::{ArrayLength, GenericArray};
use opaque_ke::ksf::Ksf;
use opaque_ke::rand::{CryptoRng, Error as RandError, RngCore};
use opaque_ke::{
    ClientLogin, ClientLoginFinishParameters, ClientRegistration,
    ClientRegistrationFinishParameters, CredentialFinalization, CredentialRequest,
    CredentialResponse, Identifiers, ServerLogin, ServerLoginParameters, ServerRegistration,
    ServerSetup,
};
use sha2::Sha512;
use zeroize::{Zeroize, Zeroizing};

use super::discovery_domain::{
    DiscoveryCryptoError, DiscoveryExchangeBinding, DiscoveryOfferBinding, PairingBundle,
};
use super::discovery_exchange::{
    ConnectorAwaitingPublisherBundle, PublisherReceivedConnectorBundle,
};
use super::discovery_signal_types::DISCOVERY_PIN_MAX_BYTES;
use super::discovery_wire::{OpaqueKe1, OpaqueKe2, OpaqueKe3ConnectorBundle};

const SESSION_KEY_BYTES: usize = 64;
const RNG_SEED_BYTES: usize = 32;

// These values, including Argon2id version and output size, are immutable
// parameters of discovery pairing suite/version 1. Changing any of them
// requires a new advertised suite version.
const ARGON2_KSF_MEMORY_KIB: u32 = 19 * 1024;
const ARGON2_KSF_ITERATIONS: u32 = 2;
const ARGON2_KSF_PARALLELISM: u32 = 1;
const ARGON2_KSF_OUTPUT_BYTES: usize = 64;
const ARGON2_KSF_SALT: [u8; argon2::RECOMMENDED_SALT_LEN] =
    [0; argon2::RECOMMENDED_SALT_LEN];

struct DiscoveryCipherSuite;

impl CipherSuite for DiscoveryCipherSuite {
    type OprfCs = opaque_ke::Ristretto255;
    type KeyExchange = opaque_ke::TripleDh<opaque_ke::Ristretto255, Sha512>;
    type Ksf = DiscoveryArgon2Ksf;
}

#[derive(Default)]
struct DiscoveryArgon2Ksf;

impl Ksf for DiscoveryArgon2Ksf {
    fn hash<L: ArrayLength<u8>>(
        &self,
        mut input: GenericArray<u8, L>,
    ) -> Result<GenericArray<u8, L>, InternalError> {
        let result = stretch_opaque_output(&input);
        let input_bytes: &mut [u8] = input.as_mut();
        input_bytes.zeroize();
        result
    }
}

fn stretch_opaque_output<L: ArrayLength<u8>>(
    input: &GenericArray<u8, L>,
) -> Result<GenericArray<u8, L>, InternalError> {
    if input.len() != ARGON2_KSF_OUTPUT_BYTES {
        return Err(InternalError::KsfError);
    }
    let params = Params::new(
        ARGON2_KSF_MEMORY_KIB,
        ARGON2_KSF_ITERATIONS,
        ARGON2_KSF_PARALLELISM,
        Some(ARGON2_KSF_OUTPUT_BYTES),
    )
    .map_err(|_| InternalError::KsfError)?;
    let mut blocks = Vec::<Block>::new();
    blocks
        .try_reserve_exact(params.block_count())
        .map_err(|_| InternalError::KsfError)?;
    blocks.resize(params.block_count(), Block::default());
    let mut memory = Zeroizing::new(blocks);
    let mut output = GenericArray::<u8, L>::default();
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

    // OPAQUE supplies a credential-specific randomized OPRF output here, not
    // the user PIN. A fixed KSF salt is therefore deterministic by design and
    // matches OPAQUE's KSF contract across registration and login.
    if argon2
        .hash_password_into_with_memory(
            &input[..],
            &ARGON2_KSF_SALT,
            &mut output[..],
            memory.as_mut_slice(),
        )
        .is_err()
    {
        output[..].zeroize();
        return Err(InternalError::KsfError);
    }
    Ok(output)
}

struct DiscoveryRng {
    inner: ChaCha20Rng,
}

impl DiscoveryRng {
    fn from_os() -> Result<Self, DiscoveryCryptoError> {
        let mut seed = Zeroizing::new([0u8; RNG_SEED_BYTES]);
        getrandom::getrandom(&mut seed[..])
            .map_err(|_| DiscoveryCryptoError::CryptographicFailure)?;
        Ok(Self {
            inner: <ChaCha20Rng as ChaChaSeedableRng>::from_seed(*seed),
        })
    }
}

impl RngCore for DiscoveryRng {
    fn next_u32(&mut self) -> u32 {
        InfallibleRng::next_u32(&mut self.inner)
    }

    fn next_u64(&mut self) -> u64 {
        InfallibleRng::next_u64(&mut self.inner)
    }

    fn fill_bytes(&mut self, destination: &mut [u8]) {
        InfallibleRng::fill_bytes(&mut self.inner, destination);
    }

    fn try_fill_bytes(&mut self, destination: &mut [u8]) -> Result<(), RandError> {
        self.fill_bytes(destination);
        Ok(())
    }
}

impl CryptoRng for DiscoveryRng {}
impl zeroize::ZeroizeOnDrop for DiscoveryRng {}

impl fmt::Debug for DiscoveryRng {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DiscoveryRng([REDACTED])")
    }
}

#[derive(Clone)]
pub struct PublisherOffer {
    binding: DiscoveryOfferBinding,
    server_setup: ServerSetup<DiscoveryCipherSuite>,
    password_file: ServerRegistration<DiscoveryCipherSuite>,
}

impl PublisherOffer {
    /// Performs OPAQUE registration locally inside the publisher process.
    ///
    /// `pin` is consumed exactly as supplied: empty and `b"0"` are valid.
    pub fn register(
        binding: DiscoveryOfferBinding,
        pin: &[u8],
    ) -> Result<Self, DiscoveryCryptoError> {
        validate_pin(pin)?;
        let mut rng = DiscoveryRng::from_os()?;
        let server_setup = ServerSetup::<DiscoveryCipherSuite>::new(&mut rng);
        let client_start = ClientRegistration::<DiscoveryCipherSuite>::start(&mut rng, pin)
            .map_err(|_| DiscoveryCryptoError::CryptographicFailure)?;
        let credential_identifier = binding.credential_identifier();
        let server_start = ServerRegistration::<DiscoveryCipherSuite>::start(
            &server_setup,
            client_start.message,
            &credential_identifier,
        )
        .map_err(|_| DiscoveryCryptoError::CryptographicFailure)?;
        let client_identifier = binding.connector_identifier();
        let server_identifier = binding.publisher_identifier();
        let identifiers = Identifiers {
            client: Some(&client_identifier),
            server: Some(&server_identifier),
        };
        let ksf = DiscoveryArgon2Ksf;
        let client_finish = client_start
            .state
            .finish(
                &mut rng,
                pin,
                server_start.message,
                ClientRegistrationFinishParameters::new(identifiers, Some(&ksf)),
            )
            .map_err(|_| DiscoveryCryptoError::CryptographicFailure)?;
        let mut registration_export_key = client_finish.export_key;
        registration_export_key.zeroize();
        let password_file = ServerRegistration::finish(client_finish.message);
        Ok(Self {
            binding,
            server_setup,
            password_file,
        })
    }

    pub fn binding(&self) -> &DiscoveryOfferBinding {
        &self.binding
    }

    pub fn start_exchange(
        &self,
        binding: DiscoveryExchangeBinding,
        ke1: OpaqueKe1,
    ) -> Result<(PublisherAwaitingKe3Bundle, OpaqueKe2), DiscoveryCryptoError> {
        if binding.offer() != &self.binding {
            return Err(DiscoveryCryptoError::BindingMismatch);
        }
        let credential_request = parse_ke1(ke1.as_bytes())?;
        let context = binding.exchange_context();
        let client_identifier = self.binding.connector_identifier();
        let server_identifier = self.binding.publisher_identifier();
        let credential_identifier = self.binding.credential_identifier();
        let parameters = ServerLoginParameters {
            context: Some(&context),
            identifiers: Identifiers {
                client: Some(&client_identifier),
                server: Some(&server_identifier),
            },
        };
        let mut rng = DiscoveryRng::from_os()?;
        let start = ServerLogin::start(
            &mut rng,
            &self.server_setup,
            Some(self.password_file.clone()),
            credential_request,
            &credential_identifier,
            parameters,
        )
        .map_err(|_| DiscoveryCryptoError::AuthenticationFailed)?;
        let ke2_bytes = start.message.serialize().to_vec();
        Ok((
            PublisherAwaitingKe3Bundle {
                binding,
                login: start.state,
                ke1: ke1.into_bytes(),
                ke2: ke2_bytes.clone(),
            },
            OpaqueKe2::from_validated(ke2_bytes),
        ))
    }
}

impl fmt::Debug for PublisherOffer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PublisherOffer")
            .field("binding", &self.binding)
            .field("opaque_registration", &"[REDACTED]")
            .finish()
    }
}

#[must_use]
pub struct ConnectorAwaitingKe2 {
    binding: DiscoveryExchangeBinding,
    pin: Zeroizing<Vec<u8>>,
    login: ClientLogin<DiscoveryCipherSuite>,
    ke1: Vec<u8>,
}

impl ConnectorAwaitingKe2 {
    /// Starts OPAQUE with the exact PIN bytes; no trimming or numeric parsing occurs.
    pub fn start(
        binding: DiscoveryExchangeBinding,
        pin: &[u8],
    ) -> Result<(Self, OpaqueKe1), DiscoveryCryptoError> {
        validate_pin(pin)?;
        let mut rng = DiscoveryRng::from_os()?;
        let start = ClientLogin::<DiscoveryCipherSuite>::start(&mut rng, pin)
            .map_err(|_| DiscoveryCryptoError::CryptographicFailure)?;
        let ke1 = start.message.serialize().to_vec();
        Ok((
            Self {
                binding,
                pin: Zeroizing::new(pin.to_vec()),
                login: start.state,
                ke1: ke1.clone(),
            },
            OpaqueKe1::from_validated(ke1),
        ))
    }

    pub fn finish(
        self,
        ke2: OpaqueKe2,
        connector_bundle: PairingBundle,
    ) -> Result<
        (ConnectorAwaitingPublisherBundle, OpaqueKe3ConnectorBundle),
        DiscoveryCryptoError,
    > {
        let Self {
            binding,
            pin,
            login,
            ke1,
        } = self;
        let credential_response = parse_ke2(ke2.as_bytes())?;
        let context = binding.exchange_context();
        let client_identifier = binding.offer().connector_identifier();
        let server_identifier = binding.offer().publisher_identifier();
        let identifiers = Identifiers {
            client: Some(&client_identifier),
            server: Some(&server_identifier),
        };
        let ksf = DiscoveryArgon2Ksf;
        let mut rng = DiscoveryRng::from_os()?;
        let finish = login
            .finish(
                &mut rng,
                pin.as_slice(),
                credential_response,
                ClientLoginFinishParameters::new(Some(&context), identifiers, Some(&ksf)),
            )
            .map_err(|_| DiscoveryCryptoError::AuthenticationFailed)?;
        let ke3 = finish.message.serialize().to_vec();
        let mut source_session_key = finish.session_key;
        let session_key = copy_session_key(source_session_key.as_slice());
        source_session_key.zeroize();
        let session_key = session_key?;
        let mut export_key = finish.export_key;
        export_key.zeroize();
        ConnectorAwaitingPublisherBundle::from_pake(
            binding,
            session_key,
            &ke1,
            ke2.as_bytes(),
            ke3,
            &connector_bundle,
        )
    }

    pub fn binding(&self) -> &DiscoveryExchangeBinding {
        &self.binding
    }
}

impl fmt::Debug for ConnectorAwaitingKe2 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConnectorAwaitingKe2")
            .field("binding", &self.binding)
            .field("pin", &"[REDACTED]")
            .field("opaque_state", &"[REDACTED]")
            .finish()
    }
}

#[must_use]
pub struct PublisherAwaitingKe3Bundle {
    binding: DiscoveryExchangeBinding,
    login: ServerLogin<DiscoveryCipherSuite>,
    ke1: Vec<u8>,
    ke2: Vec<u8>,
}

impl PublisherAwaitingKe3Bundle {
    pub fn finish(
        self,
        message: OpaqueKe3ConnectorBundle,
    ) -> Result<(PublisherReceivedConnectorBundle, PairingBundle), DiscoveryCryptoError> {
        let Self {
            binding,
            login,
            ke1,
            ke2,
        } = self;
        let finalization = parse_ke3(message.ke3())?;
        let context = binding.exchange_context();
        let client_identifier = binding.offer().connector_identifier();
        let server_identifier = binding.offer().publisher_identifier();
        let parameters = ServerLoginParameters {
            context: Some(&context),
            identifiers: Identifiers {
                client: Some(&client_identifier),
                server: Some(&server_identifier),
            },
        };
        let finish = login
            .finish(finalization, parameters)
            .map_err(|_| DiscoveryCryptoError::AuthenticationFailed)?;
        let mut source_session_key = finish.session_key;
        let session_key = copy_session_key(source_session_key.as_slice());
        source_session_key.zeroize();
        let session_key = session_key?;
        PublisherReceivedConnectorBundle::from_pake(
            binding,
            session_key,
            &ke1,
            &ke2,
            &message,
        )
    }

    pub fn binding(&self) -> &DiscoveryExchangeBinding {
        &self.binding
    }
}

impl fmt::Debug for PublisherAwaitingKe3Bundle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PublisherAwaitingKe3Bundle")
            .field("binding", &self.binding)
            .field("opaque_state", &"[REDACTED]")
            .finish()
    }
}

fn validate_pin(pin: &[u8]) -> Result<(), DiscoveryCryptoError> {
    if pin.len() > DISCOVERY_PIN_MAX_BYTES {
        return Err(DiscoveryCryptoError::PinTooLong);
    }
    Ok(())
}

fn parse_ke1(bytes: &[u8]) -> Result<CredentialRequest<DiscoveryCipherSuite>, DiscoveryCryptoError> {
    let message = CredentialRequest::deserialize(bytes)
        .map_err(|_| DiscoveryCryptoError::InvalidMessage)?;
    if message.serialize().as_slice() != bytes {
        return Err(DiscoveryCryptoError::InvalidMessage);
    }
    Ok(message)
}

fn parse_ke2(bytes: &[u8]) -> Result<CredentialResponse<DiscoveryCipherSuite>, DiscoveryCryptoError> {
    let message = CredentialResponse::deserialize(bytes)
        .map_err(|_| DiscoveryCryptoError::InvalidMessage)?;
    if message.serialize().as_slice() != bytes {
        return Err(DiscoveryCryptoError::InvalidMessage);
    }
    Ok(message)
}

fn parse_ke3(
    bytes: &[u8],
) -> Result<CredentialFinalization<DiscoveryCipherSuite>, DiscoveryCryptoError> {
    let message = CredentialFinalization::deserialize(bytes)
        .map_err(|_| DiscoveryCryptoError::InvalidMessage)?;
    if message.serialize().as_slice() != bytes {
        return Err(DiscoveryCryptoError::InvalidMessage);
    }
    Ok(message)
}

fn copy_session_key(bytes: &[u8]) -> Result<[u8; SESSION_KEY_BYTES], DiscoveryCryptoError> {
    let mut output = [0u8; SESSION_KEY_BYTES];
    if bytes.len() != output.len() {
        return Err(DiscoveryCryptoError::CryptographicFailure);
    }
    output.copy_from_slice(bytes);
    Ok(output)
}
