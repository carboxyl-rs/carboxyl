use std::io;
use std::path::Path;

use url::Url;

use color_eyre::eyre::Result;

pub fn normalize_url(raw: Option<String>) -> Result<Url> {
    let Some(raw) = raw else {
        return Ok(Url::parse("about:blank")?);
    };

    if raw.contains("://") || raw.starts_with("about:") {
        return Ok(Url::parse(&raw)?);
    }

    if Path::new(&raw).exists() {
        return Url::from_file_path(Path::new(&raw)).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("failed to convert path to file URL: {raw}"),
            )
            .into()
        });
    }

    Ok(Url::parse(&format!("https://{raw}"))?)
}
