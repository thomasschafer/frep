use anyhow::bail;

use crate::{
    search::FileSearcher,
    validation::{
        SearchConfiguration, SimpleErrorHandler, ValidationResult, validate_search_configuration,
    },
};

pub fn find_and_replace(search_config: SearchConfiguration<'_>) -> anyhow::Result<String> {
    let mut error_handler = SimpleErrorHandler::new();
    let search_config = validate_search_configuration(search_config, &mut error_handler)?;
    let searcher = match search_config {
        ValidationResult::Success(search_config) => FileSearcher::new(search_config),
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
