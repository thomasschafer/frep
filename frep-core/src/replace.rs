use std::{
    collections::HashMap,
    fs::{self, File},
    io::{BufReader, BufWriter, Write},
    path::Path,
};
use tempfile::NamedTempFile;

use crate::search::{SearchResult, SearchType};
use crate::{line_reader::BufReadExt, search};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplaceResult {
    Success,
    Error(String),
}

/// NOTE: this should only be called with search results from the same file
// TODO: enforce the above via types
pub fn replace_in_file(results: &mut [SearchResult]) -> anyhow::Result<()> {
    let file_path = match results {
        [r, ..] => r.path.clone(),
        [] => return Ok(()),
    };
    debug_assert!(results.iter().all(|r| r.path == file_path));

    let mut line_map: HashMap<_, _> = results
        .iter_mut()
        .map(|res| (res.line_number, res))
        .collect();

    let parent_dir = file_path.parent().ok_or_else(|| {
        anyhow::anyhow!(
            "Cannot create temp file: target path '{}' has no parent directory",
            file_path.display()
        )
    })?;
    let temp_output_file = NamedTempFile::new_in(parent_dir)?;

    // Scope the file operations so they're closed before rename
    {
        let input = File::open(file_path.clone())?;
        let reader = BufReader::new(input);

        let output = File::create(temp_output_file.path())?;
        let mut writer = BufWriter::new(output);

        for (mut line_number, line_result) in reader.lines_with_endings().enumerate() {
            line_number += 1; // Ensure line-number is 1-indexed
            let (mut line, line_ending) = line_result?;
            if let Some(res) = line_map.get_mut(&line_number) {
                if line == res.line.as_bytes() {
                    line = res.replacement.as_bytes().to_vec();
                    res.replace_result = Some(ReplaceResult::Success);
                } else {
                    res.replace_result = Some(ReplaceResult::Error(
                        "File changed since last search".to_owned(),
                    ));
                }
            }
            line.extend(line_ending.as_bytes());
            writer.write_all(&line)?;
        }

        writer.flush()?;
    }

    temp_output_file.persist(file_path)?;
    Ok(())
}

const MAX_FILE_SIZE: u64 = 100 * 1024 * 1024; // 100 MB

fn should_replace_in_memory(path: &Path) -> Result<bool, std::io::Error> {
    let file_size = fs::metadata(path)?.len();
    Ok(file_size <= MAX_FILE_SIZE)
}

/// Performs search and replace operations in a file
///
/// This function implements a hybrid approach to file replacements:
/// 1. For files under the `MAX_FILE_SIZE` threshold, it attempts an in-memory replacement
/// 2. If the file is large or in-memory replacement fails, it falls back to line-by-line chunked replacement
///
/// This approach optimizes for performance while maintaining reasonable memory usage limits.
///
/// # Arguments
///
/// * `file_path` - Path to the file to process
/// * `search` - The search pattern (fixed string, regex, or advanced regex)
/// * `replace` - The replacement string
///
/// # Returns
///
/// * `Ok(true)` if replacements were made in the file
/// * `Ok(false)` if no replacements were made (no matches found)
/// * `Err` if any errors occurred during the operation
pub fn replace_all_in_file(
    file_path: &Path,
    search: &SearchType,
    replace: &str,
) -> anyhow::Result<bool> {
    // Try to read into memory if not too large - if this fails, or if too large, fall back to line-by-line replacement
    if matches!(should_replace_in_memory(file_path), Ok(true)) {
        match replace_in_memory(file_path, search, replace) {
            Ok(replaced) => return Ok(replaced),
            Err(e) => {
                log::error!(
                    "Found error when attempting to replace in memory for file {path_display}: {e}",
                    path_display = file_path.display(),
                );
            }
        }
    }

    replace_chunked(file_path, search, replace)
}

fn replace_chunked(file_path: &Path, search: &SearchType, replace: &str) -> anyhow::Result<bool> {
    let mut results = search::search_file(file_path, search, replace)?;
    if !results.is_empty() {
        replace_in_file(&mut results)?;
        return Ok(true);
    }

    Ok(false)
}

fn replace_in_memory(file_path: &Path, search: &SearchType, replace: &str) -> anyhow::Result<bool> {
    let content = fs::read_to_string(file_path)?;
    if let Some(new_content) = replacement_if_match(&content, search, replace) {
        let parent_dir = file_path.parent().unwrap_or(Path::new("."));
        let mut temp_file = NamedTempFile::new_in(parent_dir)?;
        temp_file.write_all(new_content.as_bytes())?;
        temp_file.persist(file_path)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Performs a search and replace operation on a string if the pattern matches
///
/// # Arguments
///
/// * `line` - The string to search within
/// * `search` - The search pattern (fixed string, regex, or advanced regex)
/// * `replace` - The replacement string
///
/// # Returns
///
/// * `Some(String)` containing the string with replacements if matches were found
/// * `None` if no matches were found
pub fn replacement_if_match(line: &str, search: &SearchType, replace: &str) -> Option<String> {
    if line.is_empty() || search.is_empty() {
        return None;
    }

    match search {
        SearchType::Fixed(fixed_str) => {
            if line.contains(fixed_str) {
                Some(line.replace(fixed_str, replace))
            } else {
                None
            }
        }
        SearchType::Pattern(pattern) => {
            if pattern.is_match(line) {
                Some(pattern.replace_all(line, replace).to_string())
            } else {
                None
            }
        }
        SearchType::PatternAdvanced(pattern) => match pattern.is_match(line) {
            Ok(true) => Some(pattern.replace_all(line, replace).to_string()),
            _ => None,
        },
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplaceStats {
    pub num_successes: usize,
    pub errors: Vec<SearchResult>,
}

pub fn calculate_statistics<I>(results: I) -> ReplaceStats
where
    I: IntoIterator<Item = SearchResult>,
{
    let mut num_successes = 0;
    let mut errors = vec![];

    results.into_iter().for_each(|res| {
        assert!(
            res.included,
            "Expected only included results, found {res:?}"
        );
        match &res.replace_result {
            Some(ReplaceResult::Success) => {
                num_successes += 1;
            }
            None => {
                let mut res = res.clone();
                res.replace_result = Some(ReplaceResult::Error(
                    "Failed to find search result in file".to_owned(),
                ));
                errors.push(res);
            }
            Some(ReplaceResult::Error(_)) => {
                errors.push(res.clone());
            }
        }
    });

    ReplaceStats {
        num_successes,
        errors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::line_reader::LineEnding;
    use crate::search::{SearchResult, SearchType};
    use regex::Regex;
    use std::path::PathBuf;
    use tempfile::TempDir;

    // Helper functions
    fn create_search_result(
        path: &str,
        line_number: usize,
        line: &str,
        replacement: &str,
        included: bool,
        replace_result: Option<ReplaceResult>,
    ) -> SearchResult {
        SearchResult {
            path: PathBuf::from(path),
            line_number,
            line: line.to_string(),
            line_ending: LineEnding::Lf,
            replacement: replacement.to_string(),
            included,
            replace_result,
        }
    }

    fn create_test_file(temp_dir: &TempDir, name: &str, content: &str) -> PathBuf {
        let file_path = temp_dir.path().join(name);
        std::fs::write(&file_path, content).unwrap();
        file_path
    }

    fn assert_file_content(file_path: &Path, expected_content: &str) {
        let content = std::fs::read_to_string(file_path).unwrap();
        assert_eq!(content, expected_content);
    }

    fn fixed_search(pattern: &str) -> SearchType {
        SearchType::Fixed(pattern.to_string())
    }

    fn regex_search(pattern: &str) -> SearchType {
        SearchType::Pattern(Regex::new(pattern).unwrap())
    }

    // Tests for replace_in_file
    #[test]
    fn test_replace_in_file_success() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = create_test_file(
            &temp_dir,
            "test.txt",
            "line 1\nold text\nline 3\nold text\nline 5\n",
        );

        // Create search results
        let mut results = vec![
            create_search_result(
                file_path.to_str().unwrap(),
                2,
                "old text",
                "new text",
                true,
                None,
            ),
            create_search_result(
                file_path.to_str().unwrap(),
                4,
                "old text",
                "new text",
                true,
                None,
            ),
        ];

        // Perform replacement
        let result = replace_in_file(&mut results);
        assert!(result.is_ok());

        // Verify replacements were marked as successful
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].replace_result, Some(ReplaceResult::Success));
        assert_eq!(results[1].replace_result, Some(ReplaceResult::Success));

        // Verify file content
        assert_file_content(&file_path, "line 1\nnew text\nline 3\nnew text\nline 5\n");
    }

    #[test]
    fn test_replace_in_file_success_no_final_newline() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = create_test_file(
            &temp_dir,
            "test.txt",
            "line 1\nold text\nline 3\nold text\nline 5",
        );

        // Create search results
        let mut results = vec![
            create_search_result(
                file_path.to_str().unwrap(),
                2,
                "old text",
                "new text",
                true,
                None,
            ),
            create_search_result(
                file_path.to_str().unwrap(),
                4,
                "old text",
                "new text",
                true,
                None,
            ),
        ];

        // Perform replacement
        let result = replace_in_file(&mut results);
        assert!(result.is_ok());

        // Verify replacements were marked as successful
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].replace_result, Some(ReplaceResult::Success));
        assert_eq!(results[1].replace_result, Some(ReplaceResult::Success));

        // Verify file content
        let new_content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(new_content, "line 1\nnew text\nline 3\nnew text\nline 5");
    }

    #[test]
    fn test_replace_in_file_success_windows_newlines() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = create_test_file(
            &temp_dir,
            "test.txt",
            "line 1\r\nold text\r\nline 3\r\nold text\r\nline 5\r\n",
        );

        // Create search results
        let mut results = vec![
            create_search_result(
                file_path.to_str().unwrap(),
                2,
                "old text",
                "new text",
                true,
                None,
            ),
            create_search_result(
                file_path.to_str().unwrap(),
                4,
                "old text",
                "new text",
                true,
                None,
            ),
        ];

        // Perform replacement
        let result = replace_in_file(&mut results);
        assert!(result.is_ok());

        // Verify replacements were marked as successful
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].replace_result, Some(ReplaceResult::Success));
        assert_eq!(results[1].replace_result, Some(ReplaceResult::Success));

        // Verify file content
        let new_content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(
            new_content,
            "line 1\r\nnew text\r\nline 3\r\nnew text\r\nline 5\r\n"
        );
    }

    #[test]
    fn test_replace_in_file_success_mixed_newlines() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = create_test_file(
            &temp_dir,
            "test.txt",
            "\n\r\nline 1\nold text\r\nline 3\nline 4\r\nline 5\r\n\n\n",
        );

        // Create search results
        let mut results = vec![
            create_search_result(
                file_path.to_str().unwrap(),
                4,
                "old text",
                "new text",
                true,
                None,
            ),
            create_search_result(
                file_path.to_str().unwrap(),
                7,
                "line 5",
                "updated line 5",
                true,
                None,
            ),
        ];

        // Perform replacement
        let result = replace_in_file(&mut results);
        assert!(result.is_ok());

        // Verify replacements were marked as successful
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].replace_result, Some(ReplaceResult::Success));
        assert_eq!(results[1].replace_result, Some(ReplaceResult::Success));

        // Verify file content
        let new_content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(
            new_content,
            "\n\r\nline 1\nnew text\r\nline 3\nline 4\r\nupdated line 5\r\n\n\n"
        );
    }

    #[test]
    fn test_replace_in_file_line_mismatch() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = create_test_file(&temp_dir, "test.txt", "line 1\nactual text\nline 3\n");

        // Create search result with mismatching line
        let mut results = vec![create_search_result(
            file_path.to_str().unwrap(),
            2,
            "expected text",
            "new text",
            true,
            None,
        )];

        // Perform replacement
        let result = replace_in_file(&mut results);
        assert!(result.is_ok());

        // Verify replacement was marked as error
        assert_eq!(
            results[0].replace_result,
            Some(ReplaceResult::Error(
                "File changed since last search".to_owned()
            ))
        );

        // Verify file content is unchanged
        let new_content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(new_content, "line 1\nactual text\nline 3\n");
    }

    #[test]
    fn test_replace_in_file_nonexistent_file() {
        let mut results = vec![create_search_result(
            "/nonexistent/path/file.txt",
            1,
            "old",
            "new",
            true,
            None,
        )];

        let result = replace_in_file(&mut results);
        assert!(result.is_err());
    }

    #[test]
    fn test_replace_in_file_no_parent_directory() {
        let mut results = vec![SearchResult {
            path: PathBuf::from("/"),
            line_number: 0,
            line: "foo".into(),
            line_ending: LineEnding::Lf,
            replacement: "bar".into(),
            included: true,
            replace_result: None,
        }];

        let result = replace_in_file(&mut results);
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("no parent directory"));
        }
    }

    // Tests for replace_in_memory
    #[test]
    fn test_replace_in_memory() {
        let temp_dir = TempDir::new().unwrap();

        // Test with fixed string
        let file_path = create_test_file(
            &temp_dir,
            "test.txt",
            "This is a test.\nIt contains search_term that should be replaced.\nMultiple lines with search_term here.",
        );

        let result = replace_in_memory(&file_path, &fixed_search("search_term"), "replacement");
        assert!(result.is_ok());
        assert!(result.unwrap()); // Should return true for modifications

        assert_file_content(
            &file_path,
            "This is a test.\nIt contains replacement that should be replaced.\nMultiple lines with replacement here.",
        );

        // Test with regex pattern
        let regex_path = create_test_file(
            &temp_dir,
            "regex_test.txt",
            "Number: 123, Code: 456, ID: 789",
        );

        let result = replace_in_memory(&regex_path, &regex_search(r"\d{3}"), "XXX");
        assert!(result.is_ok());
        assert!(result.unwrap());

        assert_file_content(&regex_path, "Number: XXX, Code: XXX, ID: XXX");
    }

    #[test]
    fn test_replace_in_memory_no_match() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = create_test_file(
            &temp_dir,
            "no_match.txt",
            "This is a test file with no matches.",
        );

        let result = replace_in_memory(&file_path, &fixed_search("nonexistent"), "replacement");
        assert!(result.is_ok());
        assert!(!result.unwrap()); // Should return false for no modifications

        // Verify file content unchanged
        assert_file_content(&file_path, "This is a test file with no matches.");
    }

    #[test]
    fn test_replace_in_memory_empty_file() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = create_test_file(&temp_dir, "empty.txt", "");

        let result = replace_in_memory(&file_path, &fixed_search("anything"), "replacement");
        assert!(result.is_ok());
        assert!(!result.unwrap());

        // Verify file still empty
        assert_file_content(&file_path, "");
    }

    #[test]
    fn test_replace_in_memory_nonexistent_file() {
        let result = replace_in_memory(
            Path::new("/nonexistent/path/file.txt"),
            &fixed_search("test"),
            "replacement",
        );
        assert!(result.is_err());
    }

    // Tests for replace_chunked
    #[test]
    fn test_replace_chunked() {
        let temp_dir = TempDir::new().unwrap();

        // Test with fixed string
        let file_path = create_test_file(
            &temp_dir,
            "test.txt",
            "This is line one.\nThis contains search_pattern to replace.\nAnother line with search_pattern here.\nFinal line.",
        );

        let result = replace_chunked(&file_path, &fixed_search("search_pattern"), "replacement");
        assert!(result.is_ok());
        assert!(result.unwrap()); // Check that replacement happened

        assert_file_content(
            &file_path,
            "This is line one.\nThis contains replacement to replace.\nAnother line with replacement here.\nFinal line.",
        );

        // Test with regex pattern
        let regex_path = create_test_file(
            &temp_dir,
            "regex.txt",
            "Line with numbers: 123 and 456.\nAnother line with 789.",
        );

        let result = replace_chunked(&regex_path, &regex_search(r"\d{3}"), "XXX");
        assert!(result.is_ok());
        assert!(result.unwrap());

        assert_file_content(
            &regex_path,
            "Line with numbers: XXX and XXX.\nAnother line with XXX.",
        );
    }

    #[test]
    fn test_replace_chunked_no_match() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = create_test_file(
            &temp_dir,
            "test.txt",
            "This is a test file with no matching patterns.",
        );

        let result = replace_chunked(&file_path, &fixed_search("nonexistent"), "replacement");
        assert!(result.is_ok());
        assert!(!result.unwrap());

        // Verify file content unchanged
        assert_file_content(&file_path, "This is a test file with no matching patterns.");
    }

    #[test]
    fn test_replace_chunked_empty_file() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = create_test_file(&temp_dir, "empty.txt", "");

        let result = replace_chunked(&file_path, &fixed_search("anything"), "replacement");
        assert!(result.is_ok());
        assert!(!result.unwrap());

        // Verify file still empty
        assert_file_content(&file_path, "");
    }

    #[test]
    fn test_replace_chunked_nonexistent_file() {
        let result = replace_chunked(
            Path::new("/nonexistent/path/file.txt"),
            &fixed_search("test"),
            "replacement",
        );
        assert!(result.is_err());
    }

    // Tests for replace_all_in_file
    #[test]
    fn test_replace_all_in_file() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = create_test_file(
            &temp_dir,
            "test.txt",
            "This is a test file.\nIt has some content to replace.\nThe word replace should be replaced.",
        );

        let result = replace_all_in_file(&file_path, &fixed_search("replace"), "modify");
        assert!(result.is_ok());
        assert!(result.unwrap());

        assert_file_content(
            &file_path,
            "This is a test file.\nIt has some content to modify.\nThe word modify should be modifyd.",
        );
    }
}
