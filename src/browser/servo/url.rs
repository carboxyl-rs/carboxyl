use std::io;
use std::path::Path;

use url::Url;

use anyhow::Result;

pub fn normalize_url(raw: Option<String>) -> Result<Url> {
    let Some(raw) = raw else {
        return Ok(Url::parse("about:blank")?);
    };

    if raw.contains("://") || raw.starts_with("about:") {
        return Ok(Url::parse(&raw)?);
    }

    if Path::new(&raw).exists() {
        return Url::from_file_path(std::path::absolute(&raw)?).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("failed to convert path to file URL: {raw}"),
            )
            .into()
        });
    }

    Ok(Url::parse(&format!("https://{raw}"))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_files_become_file_urls() {
        let url = normalize_url(Some("Cargo.toml".into())).unwrap();
        assert_eq!(url.scheme(), "file");
        assert!(url.to_file_path().unwrap().is_file());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_urls_preserve_the_base_for_relative_assets() {
        let path = std::env::temp_dir().join(format!("carboxyl-url-{}.html", std::process::id()));
        std::os::unix::fs::symlink(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"),
            &path,
        )
        .unwrap();
        let result = normalize_url(Some(path.to_string_lossy().into_owned()));
        std::fs::remove_file(&path).unwrap();
        assert_eq!(result.unwrap().to_file_path().unwrap(), path);
    }
}
