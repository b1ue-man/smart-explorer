use serde::{Deserialize, Serialize};

use super::core::{
    b64, b64_decode, hex, public_fingerprint, random_bytes, random_hex_token, random_uuid_v4,
};

const IDENTITY_FILE: &str = "share_identity.json";
const IDENTITY_KEY_ACCOUNT: &str = "share:identity:iroh_secret";
const DIRECT_SECRET_PREFIX: &str = "share:identity:direct_secret:";

#[derive(Clone, Debug, Serialize, Deserialize)]
struct IdentityDisk {
    device_id: String,
    device_name: String,
    direct_lookup_id: String,
    public_key: String,
    fingerprint: String,
    #[serde(default)]
    node_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ShareIdentity {
    pub device_id: String,
    pub device_name: String,
    pub direct_lookup_id: String,
    pub public_key: String,
    pub fingerprint: String,
    pub node_id: String,
    pub iroh_secret: iroh::SecretKey,
}

impl ShareIdentity {
    pub fn load_or_create(default_name: String) -> Self {
        let path = crate::support_dirs::app_data_file(IDENTITY_FILE);
        let disk = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<IdentityDisk>(&s).ok());
        let secret = load_iroh_secret().unwrap_or_else(store_new_iroh_secret);
        let node_id = secret.public().to_string();
        let public_key = node_id.clone();
        let fingerprint = public_fingerprint(public_key.as_bytes());
        match disk {
            Some(d) => ShareIdentity {
                device_id: d.device_id,
                device_name: if d.device_name.trim().is_empty() {
                    default_name
                } else {
                    d.device_name
                },
                direct_lookup_id: d.direct_lookup_id,
                public_key,
                fingerprint,
                node_id,
                iroh_secret: secret,
            },
            _ => {
                let ident = Self::new(default_name, secret);
                let _ = ident.save();
                ident
            }
        }
    }

    fn new(device_name: String, iroh_secret: iroh::SecretKey) -> Self {
        let lookup_id = random_hex_token::<12>();
        let direct_secret = random_bytes::<32>();
        let _ = crate::creds::set_secret(&direct_secret_account(&lookup_id), &b64(&direct_secret));
        let node_id = iroh_secret.public().to_string();
        let public_key = node_id.clone();
        let fingerprint = public_fingerprint(public_key.as_bytes());
        ShareIdentity {
            device_id: random_uuid_v4(),
            device_name,
            direct_lookup_id: lookup_id,
            public_key,
            fingerprint,
            node_id,
            iroh_secret,
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let disk = IdentityDisk {
            device_id: self.device_id.clone(),
            device_name: self.device_name.clone(),
            direct_lookup_id: self.direct_lookup_id.clone(),
            public_key: self.public_key.clone(),
            fingerprint: self.fingerprint.clone(),
            node_id: Some(self.node_id.clone()),
        };
        let path = crate::support_dirs::app_data_file(IDENTITY_FILE);
        std::fs::create_dir_all(path.parent().unwrap_or_else(|| std::path::Path::new(".")))?;
        std::fs::write(path, serde_json::to_string_pretty(&disk).unwrap())
    }

    pub fn direct_secret(&self) -> Vec<u8> {
        crate::creds::get_secret(&direct_secret_account(&self.direct_lookup_id))
            .and_then(|s| b64_decode(&s).ok())
            .filter(|v| v.len() == 32)
            .unwrap_or_else(|| {
                let secret = random_bytes::<32>().to_vec();
                let _ = crate::creds::set_secret(
                    &direct_secret_account(&self.direct_lookup_id),
                    &b64(&secret),
                );
                secret
            })
    }

    pub fn direct_code(&self) -> String {
        format!(
            "SE-D3-{}-{}-{}-{}",
            self.direct_lookup_id,
            hex(&self.direct_secret()),
            self.fingerprint,
            self.node_id
        )
    }

    pub fn regenerate_direct_code(&mut self) {
        crate::creds::delete_secret(&direct_secret_account(&self.direct_lookup_id));
        self.direct_lookup_id = random_hex_token::<12>();
        let secret = random_bytes::<32>();
        let _ = crate::creds::set_secret(
            &direct_secret_account(&self.direct_lookup_id),
            &b64(&secret),
        );
        let _ = self.save();
    }

    pub fn set_device_name(&mut self, name: String) {
        let n = name.trim();
        if !n.is_empty() {
            self.device_name = n.to_string();
            let _ = self.save();
        }
    }
}

pub(crate) fn direct_secret_account(lookup_id: &str) -> String {
    format!("{DIRECT_SECRET_PREFIX}{lookup_id}")
}

fn load_iroh_secret() -> Option<iroh::SecretKey> {
    let raw = crate::creds::get_secret(IDENTITY_KEY_ACCOUNT)
        .and_then(|s| b64_decode(&s).ok())
        .filter(|v| v.len() == 32)?;
    let bytes: [u8; 32] = raw.try_into().ok()?;
    Some(iroh::SecretKey::from_bytes(&bytes))
}

fn store_new_iroh_secret() -> iroh::SecretKey {
    let secret = iroh::SecretKey::generate();
    let _ = crate::creds::set_secret(IDENTITY_KEY_ACCOUNT, &b64(&secret.to_bytes()));
    secret
}
