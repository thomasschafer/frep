use anyhow::bail;

use crate::validation::{
    SearchConfiguration, SimpleErrorHandler, ValidationResult, validate_search_configuration,
};

pub fn find_and_replace(search_config: SearchConfiguration<'_>) -> anyhow::Result<String> {
    let mut error_handler = SimpleErrorHandler::new();
    let result = validate_search_configuration(search_config, &mut error_handler)?;
    let searcher = match result {
        ValidationResult::Success(searcher) => searcher,
        ValidationResult::ValidationErrors => {
            bail!("{}", error_handler.errors_str().unwrap());
        }
    };

    let num_files_replaced = searcher.walk_files_and_replace(None);

    Ok(format!(
        "Success: {num_files_replaced} file{prefix} updated",
        prefix = if num_files_replaced != 1 { "s" } else { "" },
    ))
}
