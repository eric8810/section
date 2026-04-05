use anyhow::{bail, Result};
use section_core::config::{CacheConfig, SourceConfig};
use section_provider::ProviderStore;
use std::collections::HashMap;
use std::io::{self, Write};

pub fn run(store: &ProviderStore) -> Result<()> {
    println!("Welcome to Section! Let's set up your first data source.\n");

    let name = prompt("Source name", Some("default"))?;
    let provider = prompt_choice("Provider", &["fs", "s3", "webdav"])?;

    let options = match provider.as_str() {
        "fs" => prompt_fs_options()?,
        "s3" => prompt_s3_options()?,
        "webdav" => prompt_webdav_options()?,
        _ => HashMap::new(),
    };

    let source = SourceConfig {
        provider: provider.clone(),
        options,
        cache: CacheConfig::default(),
    };

    store.add_source(&name, &source)?;
    println!("\nSource '{}' added (provider: {}).", name, provider);
    println!("Run 'section mount' to mount the filesystem.");
    #[cfg(target_os = "macos")]
    println!(
        "On macOS, install macFUSE first and read docs/MACOS_ADAPTER.md before validating mount."
    );
    Ok(())
}

fn prompt(label: &str, default: Option<&str>) -> Result<String> {
    match default {
        Some(d) if !d.is_empty() => print!("{} [{}]: ", label, d),
        _ => print!("{}: ", label),
    }
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim().to_string();

    if input.is_empty() {
        match default {
            Some(d) if !d.is_empty() => Ok(d.to_string()),
            _ => bail!("{} is required", label),
        }
    } else {
        Ok(input)
    }
}

fn prompt_optional(label: &str) -> Result<Option<String>> {
    print!("{} (optional): ", label);
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim().to_string();

    if input.is_empty() {
        Ok(None)
    } else {
        Ok(Some(input))
    }
}

fn prompt_choice(label: &str, choices: &[&str]) -> Result<String> {
    let choices_str = choices.join(", ");
    print!("{} ({}): ", label, choices_str);
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim().to_string();

    if choices.contains(&input.as_str()) {
        Ok(input)
    } else {
        bail!(
            "Invalid choice '{}'. Expected one of: {}",
            input,
            choices_str
        )
    }
}

fn prompt_fs_options() -> Result<HashMap<String, String>> {
    let root = prompt("Root path", None)?;
    Ok([("root".to_string(), root)].into_iter().collect())
}

fn prompt_s3_options() -> Result<HashMap<String, String>> {
    let bucket = prompt("Bucket", None)?;
    let region = prompt("Region", Some("us-east-1"))?;
    let endpoint = prompt_optional("Endpoint")?;
    let access_key = prompt("Access Key ID", None)?;
    let secret_key = prompt("Secret Access Key", None)?;

    let mut opts = HashMap::new();
    opts.insert("bucket".to_string(), bucket);
    opts.insert("region".to_string(), region);
    if let Some(ep) = endpoint {
        opts.insert("endpoint".to_string(), ep);
    }
    opts.insert("access_key_id".to_string(), access_key);
    opts.insert("secret_access_key".to_string(), secret_key);
    Ok(opts)
}

fn prompt_webdav_options() -> Result<HashMap<String, String>> {
    let endpoint = prompt("WebDAV URL", None)?;
    let username = prompt("Username", None)?;
    let password = prompt("Password", None)?;

    let mut opts = HashMap::new();
    opts.insert("endpoint".to_string(), endpoint);
    opts.insert("username".to_string(), username);
    opts.insert("password".to_string(), password);
    Ok(opts)
}
