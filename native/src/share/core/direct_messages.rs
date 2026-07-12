use super::core::random_token;
use super::direct_protocol::{
    validate_common, validate_not_expired, DirectDecisionKind, DirectPeerIdentity,
    DirectProtocolError, SignedDirectDecision, SignedDirectDecisionReceipt, SignedDirectRequest,
    SignedDirectRequestReceipt,
};
use super::direct_transcript;

impl SignedDirectRequestReceipt {
    pub fn sign(
        request: &SignedDirectRequest,
        target: DirectPeerIdentity,
        received_at: i64,
        message: Option<String>,
        relation_secret: &[u8],
        signer: &iroh::SecretKey,
    ) -> Result<Self, DirectProtocolError> {
        let nonce = random_token(18).map_err(|_| DirectProtocolError::EntropyUnavailable)?;
        Self::sign_with_nonce(
            request,
            target,
            received_at,
            nonce,
            message,
            relation_secret,
            signer,
        )
    }

    pub fn sign_with_nonce(
        request: &SignedDirectRequest,
        target: DirectPeerIdentity,
        received_at: i64,
        nonce: impl Into<String>,
        message: Option<String>,
        relation_secret: &[u8],
        signer: &iroh::SecretKey,
    ) -> Result<Self, DirectProtocolError> {
        request.verify_at(relation_secret, received_at)?;
        require_target_matches_pin(&target, &request.target)?;
        let mut receipt = Self {
            request_id: request.request_id.clone(),
            lookup_id: request.lookup_id.clone(),
            requester: request.requester.clone(),
            target,
            request_digest: request.digest()?,
            received_at,
            expires_at: request.expires_at,
            nonce: nonce.into(),
            message,
            hmac_proof: String::new(),
            identity_signature: String::new(),
        };
        receipt.validate_fields()?;
        direct_transcript::seal_request_receipt(&mut receipt, relation_secret, signer)?;
        Ok(receipt)
    }

    pub fn verify_for(
        &self,
        request: &SignedDirectRequest,
        relation_secret: &[u8],
        now: i64,
    ) -> Result<(), DirectProtocolError> {
        self.validate_fields()?;
        validate_not_expired(now, self.received_at, self.expires_at)?;
        require_request_match(
            &self.request_id,
            &self.lookup_id,
            &self.requester,
            &self.target,
            &self.request_digest,
            request,
        )?;
        direct_transcript::verify_request_receipt(self, relation_secret)
    }

    fn validate_fields(&self) -> Result<(), DirectProtocolError> {
        validate_common(
            &self.lookup_id,
            &self.requester,
            &self.target,
            self.received_at,
            self.expires_at,
            &self.nonce,
            self.message.as_deref(),
        )?;
        require_digest(&self.request_digest)
    }
}

impl SignedDirectDecision {
    #[allow(clippy::too_many_arguments)]
    pub fn sign(
        request: &SignedDirectRequest,
        target: DirectPeerIdentity,
        decision: DirectDecisionKind,
        decision_revision: u64,
        decided_at: i64,
        expires_at: i64,
        message: Option<String>,
        relation_secret: &[u8],
        signer: &iroh::SecretKey,
    ) -> Result<Self, DirectProtocolError> {
        let nonce = random_token(18).map_err(|_| DirectProtocolError::EntropyUnavailable)?;
        Self::sign_with_nonce(
            request,
            target,
            decision,
            decision_revision,
            decided_at,
            expires_at,
            nonce,
            message,
            relation_secret,
            signer,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn sign_with_nonce(
        request: &SignedDirectRequest,
        target: DirectPeerIdentity,
        decision: DirectDecisionKind,
        decision_revision: u64,
        decided_at: i64,
        expires_at: i64,
        nonce: impl Into<String>,
        message: Option<String>,
        relation_secret: &[u8],
        signer: &iroh::SecretKey,
    ) -> Result<Self, DirectProtocolError> {
        request.validate_authenticity(relation_secret)?;
        require_target_matches_pin(&target, &request.target)?;
        if !matches!(decision, DirectDecisionKind::Revoked) && decided_at > request.expires_at {
            return Err(DirectProtocolError::Expired);
        }
        let mut signed = Self {
            request_id: request.request_id.clone(),
            lookup_id: request.lookup_id.clone(),
            requester: request.requester.clone(),
            target,
            request_digest: request.digest()?,
            decision,
            decision_revision,
            decided_at,
            expires_at,
            nonce: nonce.into(),
            message,
            hmac_proof: String::new(),
            identity_signature: String::new(),
        };
        signed.validate_fields()?;
        direct_transcript::seal_decision(&mut signed, relation_secret, signer)?;
        Ok(signed)
    }

    pub fn verify_for(
        &self,
        request: &SignedDirectRequest,
        relation_secret: &[u8],
        now: i64,
    ) -> Result<(), DirectProtocolError> {
        self.validate_fields()?;
        validate_not_expired(now, self.decided_at, self.expires_at)?;
        if !matches!(self.decision, DirectDecisionKind::Revoked)
            && self.decided_at > request.expires_at
        {
            return Err(DirectProtocolError::Expired);
        }
        require_request_match(
            &self.request_id,
            &self.lookup_id,
            &self.requester,
            &self.target,
            &self.request_digest,
            request,
        )?;
        direct_transcript::verify_decision(self, relation_secret)
    }

    pub fn digest(&self) -> Result<String, DirectProtocolError> {
        self.validate_fields()?;
        Ok(direct_transcript::decision_digest(self))
    }

    fn validate_fields(&self) -> Result<(), DirectProtocolError> {
        validate_common(
            &self.lookup_id,
            &self.requester,
            &self.target,
            self.decided_at,
            self.expires_at,
            &self.nonce,
            self.message.as_deref(),
        )?;
        require_digest(&self.request_digest)?;
        if self.decision_revision == 0 {
            return Err(DirectProtocolError::InvalidDecisionRevision);
        }
        Ok(())
    }
}

impl SignedDirectDecisionReceipt {
    pub fn sign(
        decision: &SignedDirectDecision,
        received_at: i64,
        message: Option<String>,
        relation_secret: &[u8],
        signer: &iroh::SecretKey,
    ) -> Result<Self, DirectProtocolError> {
        let nonce = random_token(18).map_err(|_| DirectProtocolError::EntropyUnavailable)?;
        Self::sign_with_nonce(
            decision,
            received_at,
            nonce,
            message,
            relation_secret,
            signer,
        )
    }

    pub fn sign_with_nonce(
        decision: &SignedDirectDecision,
        received_at: i64,
        nonce: impl Into<String>,
        message: Option<String>,
        relation_secret: &[u8],
        signer: &iroh::SecretKey,
    ) -> Result<Self, DirectProtocolError> {
        decision.validate_fields()?;
        direct_transcript::verify_decision(decision, relation_secret)?;
        validate_not_expired(received_at, decision.decided_at, decision.expires_at)?;
        let mut receipt = Self {
            request_id: decision.request_id.clone(),
            lookup_id: decision.lookup_id.clone(),
            requester: decision.requester.clone(),
            target: decision.target.clone(),
            decision_digest: decision.digest()?,
            decision: decision.decision,
            decision_revision: decision.decision_revision,
            received_at,
            expires_at: decision.expires_at,
            nonce: nonce.into(),
            message,
            hmac_proof: String::new(),
            identity_signature: String::new(),
        };
        receipt.validate_fields()?;
        direct_transcript::seal_decision_receipt(&mut receipt, relation_secret, signer)?;
        Ok(receipt)
    }

    pub fn verify_for(
        &self,
        decision: &SignedDirectDecision,
        relation_secret: &[u8],
        now: i64,
    ) -> Result<(), DirectProtocolError> {
        self.validate_fields()?;
        validate_not_expired(now, self.received_at, self.expires_at)?;
        if self.request_id != decision.request_id
            || self.lookup_id != decision.lookup_id
            || self.requester != decision.requester
            || self.target != decision.target
            || self.decision != decision.decision
            || self.decision_revision != decision.decision_revision
            || self.decision_digest != decision.digest()?
        {
            return Err(DirectProtocolError::DigestMismatch);
        }
        direct_transcript::verify_decision_receipt(self, relation_secret)
    }

    fn validate_fields(&self) -> Result<(), DirectProtocolError> {
        validate_common(
            &self.lookup_id,
            &self.requester,
            &self.target,
            self.received_at,
            self.expires_at,
            &self.nonce,
            self.message.as_deref(),
        )?;
        require_digest(&self.decision_digest)?;
        if self.decision_revision == 0 {
            return Err(DirectProtocolError::InvalidDecisionRevision);
        }
        Ok(())
    }
}

fn require_request_match(
    request_id: &super::direct_protocol::DirectRequestId,
    lookup_id: &str,
    requester: &super::direct_protocol::DirectPeerIdentity,
    target: &super::direct_protocol::DirectPeerIdentity,
    request_digest: &str,
    request: &SignedDirectRequest,
) -> Result<(), DirectProtocolError> {
    if request_id != &request.request_id
        || lookup_id != request.lookup_id
        || requester != &request.requester
        || require_target_matches_pin(target, &request.target).is_err()
        || request_digest != request.digest()?
    {
        Err(DirectProtocolError::DigestMismatch)
    } else {
        Ok(())
    }
}

fn require_target_matches_pin(
    target: &DirectPeerIdentity,
    pin: &DirectPeerIdentity,
) -> Result<(), DirectProtocolError> {
    target.validate()?;
    pin.validate_pin()?;
    if target.node_id == pin.node_id
        && target.public_key == pin.public_key
        && target.fingerprint == pin.fingerprint
    {
        Ok(())
    } else {
        Err(DirectProtocolError::IdentityKeyMismatch)
    }
}

fn require_digest(value: &str) -> Result<(), DirectProtocolError> {
    let decoded =
        super::core::b64_decode(value).map_err(|_| DirectProtocolError::DigestMismatch)?;
    if decoded.len() == 32 {
        Ok(())
    } else {
        Err(DirectProtocolError::DigestMismatch)
    }
}
