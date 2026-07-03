use std::env;

#[derive(Debug, Clone)]
pub struct EnvironmentProfile {
    pub name: String,
    pub os: String,
    pub entrypoint: String,
    pub provider: ProviderProfile,
}

#[derive(Debug, Clone)]
pub enum ProviderProfile {
    Fs,
    S3Compatible(S3Profile),
}

#[derive(Debug, Clone)]
pub struct S3Profile {
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub enable_virtual_host_style: bool,
}

#[derive(Debug, Clone)]
pub struct SourceSpec {
    pub provider: String,
    pub options: Vec<(String, String)>,
}

impl SourceSpec {
    pub fn new(provider: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            options: Vec::new(),
        }
    }

    pub fn with_option(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.options.push((key.into(), value.into()));
        self
    }
}

impl EnvironmentProfile {
    pub fn fs() -> Self {
        Self {
            name: "fs-cli".to_string(),
            os: env::consts::OS.to_string(),
            entrypoint: "section-cli".to_string(),
            provider: ProviderProfile::Fs,
        }
    }

    pub fn available() -> Vec<Self> {
        let mut profiles = vec![Self::fs()];
        if let Some(s3) = Self::from_s3_env() {
            profiles.push(s3);
        }
        profiles
    }

    pub fn from_s3_env() -> Option<Self> {
        let endpoint = env::var("SECTION_TEST_S3_ENDPOINT").ok()?;
        let bucket = env::var("SECTION_TEST_S3_BUCKET").ok()?;
        let region = env::var("SECTION_TEST_S3_REGION").unwrap_or_else(|_| "us-east-1".to_string());
        let access_key_id =
            env::var("SECTION_TEST_S3_ACCESS_KEY_ID").unwrap_or_else(|_| "test".to_string());
        let secret_access_key =
            env::var("SECTION_TEST_S3_SECRET_ACCESS_KEY").unwrap_or_else(|_| "test".to_string());
        let enable_virtual_host_style = env::var("SECTION_TEST_S3_ENABLE_VIRTUAL_HOST_STYLE")
            .map(|raw| matches!(raw.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or(false);

        Some(Self {
            name: "s3-compatible-cli".to_string(),
            os: env::consts::OS.to_string(),
            entrypoint: "section-cli".to_string(),
            provider: ProviderProfile::S3Compatible(S3Profile {
                endpoint,
                bucket,
                region,
                access_key_id,
                secret_access_key,
                enable_virtual_host_style,
            }),
        })
    }
}
