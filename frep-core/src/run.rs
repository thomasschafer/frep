use anyhow::bail;

use crate::{
    replace::replacement_if_match,
    search::FileSearcher,
    validation::{
        DirConfig, SearchConfiguration, SimpleErrorHandler, ValidationResult,
        validate_search_configuration,
    },
};

// Perform a find-and-replace recursively in a given directory
pub fn find_and_replace(
    search_config: SearchConfiguration<'_>,
    dir_config: DirConfig<'_>,
) -> anyhow::Result<String> {
    find_and_replace_impl(SearchType::Files, search_config, Some(dir_config))
}

/// Perform a find-and-replace in a string slice
pub fn find_and_replace_text(
    content: &str,
    search_config: SearchConfiguration<'_>,
) -> anyhow::Result<String> {
    find_and_replace_impl(SearchType::String(content), search_config, None)
}

enum SearchType<'a> {
    Files,
    String(&'a str),
}

#[allow(clippy::needless_pass_by_value)]
fn find_and_replace_impl(
    search_type: SearchType<'_>,
    search_config: SearchConfiguration<'_>,
    dir_config: Option<DirConfig<'_>>,
) -> anyhow::Result<String> {
    let mut error_handler = SimpleErrorHandler::new();
    let (search_config, dir_config) =
        match validate_search_configuration(search_config, dir_config, &mut error_handler)? {
            ValidationResult::Success(search_config) => search_config,
            ValidationResult::ValidationErrors => {
                bail!("{}", error_handler.errors_str().unwrap());
            }
        };

    match search_type {
        SearchType::String(content) => {
            let mut result = String::with_capacity(content.len());

            for (i, line) in content.lines().enumerate() {
                if i > 0 {
                    result.push('\n');
                }
                if let Some(replaced_line) =
                    replacement_if_match(line, &search_config.search, &search_config.replace)
                {
                    result.push_str(&replaced_line);
                } else {
                    result.push_str(line);
                }
            }
            Ok(result)
        }
        SearchType::Files => {
            let searcher = FileSearcher::new(
                search_config,
                dir_config.expect("Found None dir_config when search_type is Files"),
            );
            let num_files_replaced = searcher.walk_files_and_replace(None);

            Ok(format!(
                "Success: {num_files_replaced} file{prefix} updated",
                prefix = if num_files_replaced != 1 { "s" } else { "" },
            ))
        }
    }
}
