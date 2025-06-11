use anyhow::Error;
use ignore::overrides::OverrideBuilder;

pub fn is_regex_error(e: &Error) -> bool {
    e.downcast_ref::<regex::Error>().is_some() || e.downcast_ref::<fancy_regex::Error>().is_some()
}

pub fn add_overrides(
    overrides: &mut OverrideBuilder,
    files: &str,
    prefix: &str,
) -> anyhow::Result<()> {
    for file in files.split(',') {
        let file = file.trim();
        if !file.is_empty() {
            overrides.add(&format!("{prefix}{file}"))?;
        }
    }
    Ok(())
}
