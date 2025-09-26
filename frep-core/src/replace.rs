use std::{
    collections::HashMap,
    fs::{self, File},
    io::{BufReader, BufWriter, Write},
    path::Path,
};
use tempfile::NamedTempFile;

use crate::search::{SearchResult, SearchResultWithReplacement, SearchType};
use crate::{line_reader::BufReadExt, search};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplaceResult {
    Success,
    Error(String),
}

/// NOTE: this should only be called with search results from the same file
// TODO: enforce the above via types
pub fn replace_in_file(results: &mut [SearchResultWithReplacement]) -> anyhow::Result<()> {
    let file_path = match results {
        [r, ..] => r.search_result.path.clone(),
        [] => return Ok(()),
    };
    debug_assert!(results.iter().all(|r| r.search_result.path == file_path));

    let mut line_map = results
        .iter_mut()
        .map(|res| (res.search_result.line_number, res))
        .collect::<HashMap<_, _>>();

    let file_path = file_path.expect("File path must be present when searching in files");
    let parent_dir = file_path.parent().unwrap_or(Path::new("."));
    let temp_output_file = NamedTempFile::new_in(parent_dir)?;

    // Scope the file operations so they're closed before rename
    {
        let input = File::open(file_path.clone())?;
        let reader = BufReader::new(input);

        let output = File::create(temp_output_file.path())?;
        let mut writer = BufWriter::new(output);

        for (idx, line_result) in reader.lines_with_endings().enumerate() {
            let line_number = idx + 1; // Ensure line-number is 1-indexed
            let (mut line, line_ending) = line_result?;
            if let Some(res) = line_map.get_mut(&line_number) {
                if line == res.search_result.line.as_bytes() {
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

pub fn add_replacement(
    search_result: SearchResult,
    search: &SearchType,
    replace: &str,
) -> Option<SearchResultWithReplacement> {
    let replacement = replacement_if_match(&search_result.line, search, replace)?;
    Some(SearchResultWithReplacement {
        search_result,
        replacement,
        replace_result: None,
    })
}

fn replace_chunked(file_path: &Path, search: &SearchType, replace: &str) -> anyhow::Result<bool> {
    let search_results = search::search_file(file_path, search)?;
    if !search_results.is_empty() {
        let mut replacement_results = search_results
            .into_iter()
            .map(|r| {
                add_replacement(r, search, replace).unwrap_or_else(|| {
                    panic!("Called add_replacement with non-matching search result")
                })
            })
            .collect::<Vec<_>>();
        replace_in_file(&mut replacement_results)?;
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

    if search::contains_search(line, search) {
        let replacement = match search {
            SearchType::Fixed(fixed_str) => line.replace(fixed_str, replace),
            SearchType::Pattern(pattern) => pattern.replace_all(line, replace).to_string(),
            SearchType::PatternAdvanced(pattern) => pattern.replace_all(line, replace).to_string(),
        };
        Some(replacement)
    } else {
        None
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplaceStats {
    pub num_successes: usize,
    pub errors: Vec<SearchResultWithReplacement>,
}

pub fn calculate_statistics<I>(results: I) -> ReplaceStats
where
    I: IntoIterator<Item = SearchResultWithReplacement>,
{
    let mut num_successes = 0;
    let mut errors = vec![];

    results.into_iter().for_each(|res| {
        assert!(
            res.search_result.included,
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
    use crate::search::{SearchResult, SearchType, search_file};
    use regex::Regex;
    use std::path::PathBuf;
    use tempfile::TempDir;

    mod test_helpers {
        use crate::search::SearchType;

        pub fn create_fixed_search(term: &str) -> SearchType {
            SearchType::Fixed(term.to_string())
        }
    }

    // Helper functions
    fn create_search_result_with_replacement(
        path: &str,
        line_number: usize,
        line: &str,
        replacement: &str,
        included: bool,
        replace_result: Option<ReplaceResult>,
    ) -> SearchResultWithReplacement {
        SearchResultWithReplacement {
            search_result: SearchResult {
                path: Some(PathBuf::from(path)),
                line_number,
                line: line.to_string(),
                line_ending: LineEnding::Lf,
                included,
            },
            replacement: replacement.to_string(),
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
            create_search_result_with_replacement(
                file_path.to_str().unwrap(),
                2,
                "old text",
                "new text",
                true,
                None,
            ),
            create_search_result_with_replacement(
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
            create_search_result_with_replacement(
                file_path.to_str().unwrap(),
                2,
                "old text",
                "new text",
                true,
                None,
            ),
            create_search_result_with_replacement(
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
            create_search_result_with_replacement(
                file_path.to_str().unwrap(),
                2,
                "old text",
                "new text",
                true,
                None,
            ),
            create_search_result_with_replacement(
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
            create_search_result_with_replacement(
                file_path.to_str().unwrap(),
                4,
                "old text",
                "new text",
                true,
                None,
            ),
            create_search_result_with_replacement(
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
        let mut results = vec![create_search_result_with_replacement(
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
        let mut results = vec![create_search_result_with_replacement(
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
    fn test_replace_directory_errors() {
        let mut results = vec![create_search_result_with_replacement(
            "/", 0, "foo", "bar", true, None,
        )];

        let result = replace_in_file(&mut results);
        assert!(result.is_err());
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

    #[test]
    fn test_unicode_in_file() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "Line with Greek: αβγδε").unwrap();
        write!(temp_file, "Line with Emoji: 😀 🚀 🌍\r\n").unwrap();
        write!(temp_file, "Line with Arabic: مرحبا بالعالم").unwrap();
        temp_file.flush().unwrap();

        let search = SearchType::Pattern(Regex::new(r"\p{Greek}+").unwrap());
        let replacement = "GREEK";
        let results = search_file(temp_file.path(), &search)
            .unwrap()
            .into_iter()
            .filter_map(|r| add_replacement(r, &search, replacement))
            .collect::<Vec<_>>();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].replacement, "Line with Greek: GREEK");

        let search = SearchType::Pattern(Regex::new(r"🚀").unwrap());
        let replacement = "ROCKET";
        let results = search_file(temp_file.path(), &search)
            .unwrap()
            .into_iter()
            .filter_map(|r| add_replacement(r, &search, replacement))
            .collect::<Vec<_>>();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].replacement, "Line with Emoji: 😀 ROCKET 🌍");
        assert_eq!(results[0].search_result.line_ending, LineEnding::CrLf);
    }

    mod search_file_tests {
        use super::*;
        use fancy_regex::Regex as FancyRegex;
        use regex::Regex;
        use std::io::Write;
        use tempfile::NamedTempFile;

        #[test]
        fn test_search_file_simple_match() {
            let mut temp_file = NamedTempFile::new().unwrap();
            writeln!(temp_file, "line 1").unwrap();
            writeln!(temp_file, "search target").unwrap();
            writeln!(temp_file, "line 3").unwrap();
            temp_file.flush().unwrap();

            let search = test_helpers::create_fixed_search("search");
            let replacement = "replace";
            let results = search_file(temp_file.path(), &search)
                .unwrap()
                .into_iter()
                .filter_map(|r| add_replacement(r, &search, replacement))
                .collect::<Vec<_>>();

            assert_eq!(results.len(), 1);
            assert_eq!(results[0].search_result.line_number, 2);
            assert_eq!(results[0].search_result.line, "search target");
            assert_eq!(results[0].replacement, "replace target");
            assert!(results[0].search_result.included);
        }

        #[test]
        fn test_search_file_multiple_matches() {
            let mut temp_file = NamedTempFile::new().unwrap();
            writeln!(temp_file, "test line 1").unwrap();
            writeln!(temp_file, "test line 2").unwrap();
            writeln!(temp_file, "no match here").unwrap();
            writeln!(temp_file, "test line 4").unwrap();
            temp_file.flush().unwrap();

            let search = test_helpers::create_fixed_search("test");
            let replacement = "replaced";
            let results = search_file(temp_file.path(), &search)
                .unwrap()
                .into_iter()
                .filter_map(|r| add_replacement(r, &search, replacement))
                .collect::<Vec<_>>();

            assert_eq!(results.len(), 3);
            assert_eq!(results[0].search_result.line_number, 1);
            assert_eq!(results[0].replacement, "replaced line 1");
            assert_eq!(results[1].search_result.line_number, 2);
            assert_eq!(results[1].replacement, "replaced line 2");
            assert_eq!(results[2].search_result.line_number, 4);
            assert_eq!(results[2].replacement, "replaced line 4");
        }

        #[test]
        fn test_search_file_no_matches() {
            let mut temp_file = NamedTempFile::new().unwrap();
            writeln!(temp_file, "line 1").unwrap();
            writeln!(temp_file, "line 2").unwrap();
            writeln!(temp_file, "line 3").unwrap();
            temp_file.flush().unwrap();

            let search = SearchType::Fixed("nonexistent".to_string());
            let replacement = "replace";
            let results = search_file(temp_file.path(), &search)
                .unwrap()
                .into_iter()
                .filter_map(|r| add_replacement(r, &search, replacement))
                .collect::<Vec<_>>();

            assert_eq!(results.len(), 0);
        }

        #[test]
        fn test_search_file_regex_pattern() {
            let mut temp_file = NamedTempFile::new().unwrap();
            writeln!(temp_file, "number: 123").unwrap();
            writeln!(temp_file, "text without numbers").unwrap();
            writeln!(temp_file, "another number: 456").unwrap();
            temp_file.flush().unwrap();

            let search = SearchType::Pattern(Regex::new(r"\d+").unwrap());
            let replacement = "XXX";
            let results = search_file(temp_file.path(), &search)
                .unwrap()
                .into_iter()
                .filter_map(|r| add_replacement(r, &search, replacement))
                .collect::<Vec<_>>();

            assert_eq!(results.len(), 2);
            assert_eq!(results[0].replacement, "number: XXX");
            assert_eq!(results[1].replacement, "another number: XXX");
        }

        #[test]
        fn test_search_file_advanced_regex_pattern() {
            let mut temp_file = NamedTempFile::new().unwrap();
            writeln!(temp_file, "123abc456").unwrap();
            writeln!(temp_file, "abc").unwrap();
            writeln!(temp_file, "789xyz123").unwrap();
            writeln!(temp_file, "no match").unwrap();
            temp_file.flush().unwrap();

            // Positive lookbehind and lookahead
            let search =
                SearchType::PatternAdvanced(FancyRegex::new(r"(?<=\d{3})abc(?=\d{3})").unwrap());
            let replacement = "REPLACED";
            let results = search_file(temp_file.path(), &search)
                .unwrap()
                .into_iter()
                .filter_map(|r| add_replacement(r, &search, replacement))
                .collect::<Vec<_>>();

            assert_eq!(results.len(), 1);
            assert_eq!(results[0].replacement, "123REPLACED456");
            assert_eq!(results[0].search_result.line_number, 1);
        }

        #[test]
        fn test_search_file_empty_search() {
            let mut temp_file = NamedTempFile::new().unwrap();
            writeln!(temp_file, "some content").unwrap();
            temp_file.flush().unwrap();

            let search = SearchType::Fixed("".to_string());
            let replacement = "replace";
            let results = search_file(temp_file.path(), &search)
                .unwrap()
                .into_iter()
                .filter_map(|r| add_replacement(r, &search, replacement))
                .collect::<Vec<_>>();

            assert_eq!(results.len(), 0);
        }

        #[test]
        fn test_search_file_preserves_line_endings() {
            let mut temp_file = NamedTempFile::new().unwrap();
            write!(temp_file, "line1\nline2\r\nline3").unwrap();
            temp_file.flush().unwrap();

            let search = SearchType::Fixed("line".to_string());
            let replacement = "X";
            let results = search_file(temp_file.path(), &search)
                .unwrap()
                .into_iter()
                .filter_map(|r| add_replacement(r, &search, replacement))
                .collect::<Vec<_>>();

            assert_eq!(results.len(), 3);
            assert_eq!(results[0].search_result.line_ending, LineEnding::Lf);
            assert_eq!(results[1].search_result.line_ending, LineEnding::CrLf);
            assert_eq!(results[2].search_result.line_ending, LineEnding::None);
        }

        #[test]
        fn test_search_file_nonexistent() {
            let nonexistent_path = PathBuf::from("/this/file/does/not/exist.txt");
            let search = test_helpers::create_fixed_search("test");
            let results = search_file(&nonexistent_path, &search);
            assert!(results.is_err());
        }

        #[test]
        fn test_search_file_unicode_content() {
            let mut temp_file = NamedTempFile::new().unwrap();
            writeln!(temp_file, "Hello 世界!").unwrap();
            writeln!(temp_file, "Здравствуй мир!").unwrap();
            writeln!(temp_file, "🚀 Rocket").unwrap();
            temp_file.flush().unwrap();

            let search = SearchType::Fixed("世界".to_string());
            let replacement = "World";
            let results = search_file(temp_file.path(), &search)
                .unwrap()
                .into_iter()
                .filter_map(|r| add_replacement(r, &search, replacement))
                .collect::<Vec<_>>();

            assert_eq!(results.len(), 1);
            assert_eq!(results[0].replacement, "Hello World!");
        }

        #[test]
        fn test_search_file_with_binary_content() {
            let mut temp_file = NamedTempFile::new().unwrap();
            // Write some binary data (null bytes and other control characters)
            let binary_data = [0x00, 0x01, 0x02, 0xFF, 0xFE];
            temp_file.write_all(&binary_data).unwrap();
            temp_file.flush().unwrap();

            let search = test_helpers::create_fixed_search("test");
            let replacement = "replace";
            let results = search_file(temp_file.path(), &search)
                .unwrap()
                .into_iter()
                .filter_map(|r| add_replacement(r, &search, replacement))
                .collect::<Vec<_>>();

            assert_eq!(results.len(), 0);
        }

        #[test]
        fn test_search_file_large_content() {
            let mut temp_file = NamedTempFile::new().unwrap();

            // Write a large file with search targets scattered throughout
            for i in 0..1000 {
                if i % 100 == 0 {
                    writeln!(temp_file, "target line {i}").unwrap();
                } else {
                    writeln!(temp_file, "normal line {i}").unwrap();
                }
            }
            temp_file.flush().unwrap();

            let search = SearchType::Fixed("target".to_string());
            let replacement = "found";
            let results = search_file(temp_file.path(), &search)
                .unwrap()
                .into_iter()
                .filter_map(|r| add_replacement(r, &search, replacement))
                .collect::<Vec<_>>();

            assert_eq!(results.len(), 10); // Lines 0, 100, 200, ..., 900
            assert_eq!(results[0].search_result.line_number, 1); // 1-indexed
            assert_eq!(results[1].search_result.line_number, 101);
            assert_eq!(results[9].search_result.line_number, 901);
        }
    }

    mod replace_if_match_tests {
        use crate::validation::SearchConfig;

        use super::*;

        mod test_helpers {
            use crate::{
                search::ParsedSearchConfig,
                validation::{
                    SearchConfig, SimpleErrorHandler, ValidationResult,
                    validate_search_configuration,
                },
            };

            pub fn must_parse_search_config(search_config: SearchConfig<'_>) -> ParsedSearchConfig {
                let mut error_handler = SimpleErrorHandler::new();
                let (search_config, _dir_config) =
                    match validate_search_configuration(search_config, None, &mut error_handler)
                        .unwrap()
                    {
                        ValidationResult::Success(search_config) => search_config,
                        ValidationResult::ValidationErrors => {
                            panic!("{}", error_handler.errors_str().unwrap());
                        }
                    };
                search_config
            }
        }

        mod fixed_string_tests {
            use super::*;

            mod whole_word_true_match_case_true {

                use super::*;

                #[test]
                fn test_basic_replacement() {
                    let search_config = SearchConfig {
                        search_text: "world",
                        fixed_strings: true,
                        match_whole_word: true,
                        match_case: true,
                        replacement_text: "earth",
                        advanced_regex: false,
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match("hello world", &parsed.search, &parsed.replace),
                        Some("hello earth".to_string())
                    );
                }

                #[test]
                fn test_case_sensitivity() {
                    let search_config = SearchConfig {
                        search_text: "world",
                        fixed_strings: true,
                        match_whole_word: true,
                        match_case: true,
                        replacement_text: "earth",
                        advanced_regex: false,
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match("hello WORLD", &parsed.search, &parsed.replace),
                        None
                    );
                }

                #[test]
                fn test_word_boundaries() {
                    let search_config = SearchConfig {
                        search_text: "world",
                        fixed_strings: true,
                        match_whole_word: true,
                        match_case: true,
                        replacement_text: "earth",
                        advanced_regex: false,
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match("worldwide", &parsed.search, &parsed.replace),
                        None
                    );
                }
            }

            mod whole_word_true_match_case_false {
                use super::*;

                #[test]
                fn test_basic_replacement() {
                    let search_config = SearchConfig {
                        search_text: "world",
                        fixed_strings: true,
                        match_whole_word: true,
                        match_case: false,
                        replacement_text: "earth",
                        advanced_regex: false,
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match("hello world", &parsed.search, &parsed.replace),
                        Some("hello earth".to_string())
                    );
                }

                #[test]
                fn test_case_insensitivity() {
                    let search_config = SearchConfig {
                        search_text: "world",
                        fixed_strings: true,
                        match_whole_word: true,
                        match_case: false,
                        replacement_text: "earth",
                        advanced_regex: false,
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match("hello WORLD", &parsed.search, &parsed.replace),
                        Some("hello earth".to_string())
                    );
                }

                #[test]
                fn test_word_boundaries() {
                    let search_config = SearchConfig {
                        search_text: "world",
                        fixed_strings: true,
                        match_whole_word: true,
                        match_case: false,
                        replacement_text: "earth",
                        advanced_regex: false,
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match("worldwide", &parsed.search, &parsed.replace),
                        None
                    );
                }

                #[test]
                fn test_unicode() {
                    let search_config = SearchConfig {
                        search_text: "café",
                        fixed_strings: true,
                        match_whole_word: true,
                        match_case: false,
                        replacement_text: "restaurant",
                        advanced_regex: false,
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match("Hello CAFÉ table", &parsed.search, &parsed.replace),
                        Some("Hello restaurant table".to_string())
                    );
                }
            }

            mod whole_word_false_match_case_true {
                use super::*;

                #[test]
                fn test_basic_replacement() {
                    let search_config = SearchConfig {
                        search_text: "world",
                        fixed_strings: true,
                        match_whole_word: false,
                        match_case: true,
                        replacement_text: "earth",
                        advanced_regex: false,
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match("hello world", &parsed.search, &parsed.replace),
                        Some("hello earth".to_string())
                    );
                }

                #[test]
                fn test_case_sensitivity() {
                    let search_config = SearchConfig {
                        search_text: "world",
                        fixed_strings: true,
                        match_whole_word: false,
                        match_case: true,
                        replacement_text: "earth",
                        advanced_regex: false,
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match("hello WORLD", &parsed.search, &parsed.replace),
                        None
                    );
                }

                #[test]
                fn test_substring_matches() {
                    let search_config = SearchConfig {
                        search_text: "world",
                        fixed_strings: true,
                        match_whole_word: false,
                        match_case: true,
                        replacement_text: "earth",
                        advanced_regex: false,
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match("worldwide", &parsed.search, &parsed.replace),
                        Some("earthwide".to_string())
                    );
                }
            }

            mod whole_word_false_match_case_false {
                use super::*;

                #[test]
                fn test_basic_replacement() {
                    let search_config = SearchConfig {
                        search_text: "world",
                        fixed_strings: true,
                        match_whole_word: false,
                        match_case: false,
                        replacement_text: "earth",
                        advanced_regex: false,
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match("hello world", &parsed.search, &parsed.replace),
                        Some("hello earth".to_string())
                    );
                }

                #[test]
                fn test_case_insensitivity() {
                    let search_config = SearchConfig {
                        search_text: "world",
                        fixed_strings: true,
                        match_whole_word: false,
                        match_case: false,
                        replacement_text: "earth",
                        advanced_regex: false,
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match("hello WORLD", &parsed.search, &parsed.replace),
                        Some("hello earth".to_string())
                    );
                }

                #[test]
                fn test_substring_matches() {
                    let search_config = SearchConfig {
                        search_text: "world",
                        fixed_strings: true,
                        match_whole_word: false,
                        match_case: false,
                        replacement_text: "earth",
                        advanced_regex: false,
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match("WORLDWIDE", &parsed.search, &parsed.replace),
                        Some("earthWIDE".to_string())
                    );
                }
            }
        }

        mod regex_pattern_tests {
            use super::*;

            mod whole_word_true_match_case_true {
                use crate::validation::SearchConfig;

                use super::*;

                #[test]
                fn test_basic_regex() {
                    let re_str = r"w\w+d";
                    let search_config = SearchConfig {
                        search_text: re_str,
                        fixed_strings: false,
                        match_whole_word: true,
                        match_case: true,
                        replacement_text: "earth",
                        advanced_regex: false,
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match("hello world", &parsed.search, &parsed.replace),
                        Some("hello earth".to_string())
                    );
                }

                #[test]
                fn test_case_sensitivity() {
                    let re_str = r"world";
                    let search_config = SearchConfig {
                        search_text: re_str,
                        fixed_strings: false,
                        match_whole_word: true,
                        match_case: true,
                        replacement_text: "earth",
                        advanced_regex: false,
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match("hello WORLD", &parsed.search, &parsed.replace),
                        None
                    );
                }

                #[test]
                fn test_word_boundaries() {
                    let re_str = r"world";
                    let search_config = SearchConfig {
                        search_text: re_str,
                        fixed_strings: false,
                        match_whole_word: true,
                        match_case: true,
                        replacement_text: "earth",
                        advanced_regex: false,
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match("worldwide", &parsed.search, &parsed.replace),
                        None
                    );
                }
            }

            mod whole_word_true_match_case_false {
                use super::*;

                #[test]
                fn test_basic_regex() {
                    let re_str = r"w\w+d";
                    let search_config = SearchConfig {
                        search_text: re_str,
                        fixed_strings: false,
                        match_whole_word: true,
                        match_case: false,
                        replacement_text: "earth",
                        advanced_regex: false,
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match("hello WORLD", &parsed.search, &parsed.replace),
                        Some("hello earth".to_string())
                    );
                }

                #[test]
                fn test_word_boundaries() {
                    let re_str = r"world";
                    let search_config = SearchConfig {
                        search_text: re_str,
                        fixed_strings: false,
                        match_whole_word: true,
                        match_case: false,
                        replacement_text: "earth",
                        advanced_regex: false,
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match("worldwide", &parsed.search, &parsed.replace),
                        None
                    );
                }

                #[test]
                fn test_special_characters() {
                    let re_str = r"\d+";
                    let search_config = SearchConfig {
                        search_text: re_str,
                        fixed_strings: false,
                        match_whole_word: true,
                        match_case: false,
                        replacement_text: "NUM",
                        advanced_regex: false,
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match("test 123 number", &parsed.search, &parsed.replace),
                        Some("test NUM number".to_string())
                    );
                }

                #[test]
                fn test_unicode_word_boundaries() {
                    let re_str = r"\b\p{Script=Han}{2}\b";
                    let search_config = SearchConfig {
                        search_text: re_str,
                        fixed_strings: false,
                        match_whole_word: true,
                        match_case: false,
                        replacement_text: "XX",
                        advanced_regex: false,
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert!(
                        replacement_if_match("Text 世界 more", &parsed.search, &parsed.replace)
                            .is_some()
                    );
                    assert!(replacement_if_match("Text世界more", &parsed.search, "XX").is_none());
                }
            }

            mod whole_word_false_match_case_true {
                use super::*;

                #[test]
                fn test_basic_regex() {
                    let re_str = r"w\w+d";
                    let search_config = SearchConfig {
                        search_text: re_str,
                        fixed_strings: false,
                        match_whole_word: false,
                        match_case: true,
                        replacement_text: "earth",
                        advanced_regex: false,
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match("hello world", &parsed.search, &parsed.replace),
                        Some("hello earth".to_string())
                    );
                }

                #[test]
                fn test_case_sensitivity() {
                    let re_str = r"world";
                    let search_config = SearchConfig {
                        search_text: re_str,
                        fixed_strings: false,
                        match_whole_word: false,
                        match_case: true,
                        replacement_text: "earth",
                        advanced_regex: false,
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match("hello WORLD", &parsed.search, &parsed.replace),
                        None
                    );
                }

                #[test]
                fn test_substring_matches() {
                    let re_str = r"world";
                    let search_config = SearchConfig {
                        search_text: re_str,
                        fixed_strings: false,
                        match_whole_word: false,
                        match_case: true,
                        replacement_text: "earth",
                        advanced_regex: false,
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match("worldwide", &parsed.search, &parsed.replace),
                        Some("earthwide".to_string())
                    );
                }
            }

            mod whole_word_false_match_case_false {
                use super::*;

                #[test]
                fn test_basic_regex() {
                    let re_str = r"w\w+d";
                    let search_config = SearchConfig {
                        search_text: re_str,
                        fixed_strings: false,
                        match_whole_word: false,
                        match_case: false,
                        replacement_text: "earth",
                        advanced_regex: false,
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match("hello WORLD", &parsed.search, &parsed.replace),
                        Some("hello earth".to_string())
                    );
                }

                #[test]
                fn test_substring_matches() {
                    let re_str = r"world";
                    let search_config = SearchConfig {
                        search_text: re_str,
                        fixed_strings: false,
                        match_whole_word: false,
                        match_case: false,
                        replacement_text: "earth",
                        advanced_regex: false,
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match("WORLDWIDE", &parsed.search, &parsed.replace),
                        Some("earthWIDE".to_string())
                    );
                }

                #[test]
                fn test_complex_pattern() {
                    let re_str = r"\d{3}-\d{2}-\d{4}";
                    let search_config = SearchConfig {
                        search_text: re_str,
                        fixed_strings: false,
                        match_whole_word: false,
                        match_case: false,
                        replacement_text: "XXX-XX-XXXX",
                        advanced_regex: false,
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match("SSN: 123-45-6789", &parsed.search, &parsed.replace),
                        Some("SSN: XXX-XX-XXXX".to_string())
                    );
                }
            }
        }

        mod fancy_regex_pattern_tests {
            use super::*;

            mod whole_word_true_match_case_true {

                use super::*;

                #[test]
                fn test_lookbehind() {
                    let re_str = r"(?<=@)\w+";
                    let search_config = SearchConfig {
                        search_text: re_str,
                        match_whole_word: true,
                        fixed_strings: false,
                        advanced_regex: true,
                        match_case: true,
                        replacement_text: "domain",
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match(
                            "email: user@example.com",
                            &parsed.search,
                            &parsed.replace
                        ),
                        Some("email: user@domain.com".to_string())
                    );
                }

                #[test]
                fn test_lookahead() {
                    let re_str = r"\w+(?=\.\w+$)";
                    let search_config = SearchConfig {
                        search_text: re_str,
                        match_whole_word: true,
                        fixed_strings: false,
                        advanced_regex: true,
                        match_case: true,
                        replacement_text: "report",
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match("file: document.pdf", &parsed.search, &parsed.replace),
                        Some("file: report.pdf".to_string())
                    );
                }

                #[test]
                fn test_case_sensitivity() {
                    let re_str = r"world";
                    let search_config = SearchConfig {
                        search_text: re_str,
                        match_whole_word: true,
                        fixed_strings: false,
                        advanced_regex: true,
                        match_case: true,
                        replacement_text: "earth",
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match("hello WORLD", &parsed.search, &parsed.replace),
                        None
                    );
                }
            }

            mod whole_word_true_match_case_false {
                use super::*;

                #[test]
                fn test_lookbehind_case_insensitive() {
                    let re_str = r"(?<=@)\w+";
                    let search_config = SearchConfig {
                        search_text: re_str,
                        match_whole_word: true,
                        fixed_strings: false,
                        advanced_regex: true,
                        match_case: false,
                        replacement_text: "domain",
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match(
                            "email: user@EXAMPLE.com",
                            &parsed.search,
                            &parsed.replace
                        ),
                        Some("email: user@domain.com".to_string())
                    );
                }

                #[test]
                fn test_word_boundaries() {
                    let re_str = r"world";
                    let search_config = SearchConfig {
                        search_text: re_str,
                        match_whole_word: true,
                        fixed_strings: false,
                        advanced_regex: true,
                        match_case: false,
                        replacement_text: "earth",
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match("worldwide", &parsed.search, &parsed.replace),
                        None
                    );
                }
            }

            mod whole_word_false_match_case_true {
                use super::*;

                #[test]
                fn test_complex_pattern() {
                    let re_str = r"(?<=\d{4}-\d{2}-\d{2}T)\d{2}:\d{2}";
                    let search_config = SearchConfig {
                        search_text: re_str,
                        match_whole_word: false,
                        fixed_strings: false,
                        advanced_regex: true,
                        match_case: true,
                        replacement_text: "XX:XX",
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match(
                            "Timestamp: 2023-01-15T14:30:00Z",
                            &parsed.search,
                            &parsed.replace
                        ),
                        Some("Timestamp: 2023-01-15TXX:XX:00Z".to_string())
                    );
                }

                #[test]
                fn test_case_sensitivity() {
                    let re_str = r"WORLD";
                    let search_config = SearchConfig {
                        search_text: re_str,
                        match_whole_word: false,
                        fixed_strings: false,
                        advanced_regex: true,
                        match_case: true,
                        replacement_text: "earth",
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match("hello world", &parsed.search, &parsed.replace),
                        None
                    );
                }
            }

            mod whole_word_false_match_case_false {
                use super::*;

                #[test]
                fn test_complex_pattern_case_insensitive() {
                    let re_str = r"(?<=\[)\w+(?=\])";
                    let search_config = SearchConfig {
                        search_text: re_str,
                        match_whole_word: false,
                        fixed_strings: false,
                        advanced_regex: true,
                        match_case: false,
                        replacement_text: "ERROR",
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match(
                            "Tag: [WARNING] message",
                            &parsed.search,
                            &parsed.replace
                        ),
                        Some("Tag: [ERROR] message".to_string())
                    );
                }

                #[test]
                fn test_unicode_support() {
                    let re_str = r"\p{Greek}+";
                    let search_config = SearchConfig {
                        search_text: re_str,
                        match_whole_word: false,
                        fixed_strings: false,
                        advanced_regex: true,
                        match_case: false,
                        replacement_text: "GREEK",
                    };
                    let parsed = test_helpers::must_parse_search_config(search_config);

                    assert_eq!(
                        replacement_if_match("Symbol: αβγδ", &parsed.search, &parsed.replace),
                        Some("Symbol: GREEK".to_string())
                    );
                }
            }
        }

        #[test]
        fn test_multiple_replacements() {
            let search_config = SearchConfig {
                search_text: "world",
                fixed_strings: true,
                match_whole_word: true,
                match_case: false,
                replacement_text: "earth",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("world hello world", &parsed.search, &parsed.replace),
                Some("earth hello earth".to_string())
            );
        }

        #[test]
        fn test_no_match() {
            let search_config = SearchConfig {
                search_text: "world",
                fixed_strings: true,
                match_whole_word: true,
                match_case: false,
                replacement_text: "earth",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("worldwide", &parsed.search, &parsed.replace),
                None
            );
            let search_config = SearchConfig {
                search_text: "world",
                fixed_strings: true,
                match_whole_word: true,
                match_case: false,
                replacement_text: "earth",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("_world_", &parsed.search, &parsed.replace),
                None
            );
        }

        #[test]
        fn test_word_boundaries() {
            let search_config = SearchConfig {
                search_text: "world",
                fixed_strings: true,
                match_whole_word: true,
                match_case: false,
                replacement_text: "earth",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match(",world-", &parsed.search, &parsed.replace),
                Some(",earth-".to_string())
            );
            let search_config = SearchConfig {
                search_text: "world",
                fixed_strings: true,
                match_whole_word: true,
                match_case: false,
                replacement_text: "earth",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("world-word", &parsed.search, &parsed.replace),
                Some("earth-word".to_string())
            );
            let search_config = SearchConfig {
                search_text: "world",
                fixed_strings: true,
                match_whole_word: true,
                match_case: false,
                replacement_text: "earth",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("Hello-world!", &parsed.search, &parsed.replace),
                Some("Hello-earth!".to_string())
            );
        }

        #[test]
        fn test_case_sensitive() {
            let search_config = SearchConfig {
                search_text: "world",
                fixed_strings: true,
                match_whole_word: true,
                match_case: true,
                replacement_text: "earth",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("Hello WORLD", &parsed.search, &parsed.replace),
                None
            );
            let search_config = SearchConfig {
                search_text: "wOrld",
                fixed_strings: true,
                match_whole_word: true,
                match_case: true,
                replacement_text: "earth",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("Hello world", &parsed.search, &parsed.replace),
                None
            );
        }

        #[test]
        fn test_empty_strings() {
            let search_config = SearchConfig {
                search_text: "world",
                fixed_strings: true,
                match_whole_word: true,
                match_case: false,
                replacement_text: "earth",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("", &parsed.search, &parsed.replace),
                None
            );
            let search_config = SearchConfig {
                search_text: "",
                fixed_strings: true,
                match_whole_word: true,
                match_case: false,
                replacement_text: "earth",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("hello world", &parsed.search, &parsed.replace),
                None
            );
        }

        #[test]
        fn test_substring_no_match() {
            let search_config = SearchConfig {
                search_text: "world",
                fixed_strings: true,
                match_whole_word: true,
                match_case: false,
                replacement_text: "earth",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("worldwide web", &parsed.search, &parsed.replace),
                None
            );
            let search_config = SearchConfig {
                search_text: "world",
                fixed_strings: true,
                match_whole_word: true,
                match_case: false,
                replacement_text: "earth",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("underworld", &parsed.search, &parsed.replace),
                None
            );
        }

        #[test]
        fn test_special_regex_chars() {
            let search_config = SearchConfig {
                search_text: "(world)",
                fixed_strings: true,
                match_whole_word: true,
                match_case: false,
                replacement_text: "earth",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("hello (world)", &parsed.search, &parsed.replace),
                Some("hello earth".to_string())
            );
            let search_config = SearchConfig {
                search_text: "world.*",
                fixed_strings: true,
                match_whole_word: true,
                match_case: false,
                replacement_text: "ea+rth",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("hello world.*", &parsed.search, &parsed.replace),
                Some("hello ea+rth".to_string())
            );
        }

        #[test]
        fn test_basic_regex_patterns() {
            let re_str = r"ax*b";
            let search_config = SearchConfig {
                search_text: re_str,
                fixed_strings: false,
                match_whole_word: true,
                match_case: false,
                replacement_text: "NEW",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("foo axxxxb bar", &parsed.search, &parsed.replace),
                Some("foo NEW bar".to_string())
            );
            let search_config = SearchConfig {
                search_text: re_str,
                fixed_strings: false,
                match_whole_word: true,
                match_case: false,
                replacement_text: "NEW",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("fooaxxxxb bar", &parsed.search, &parsed.replace),
                None
            );
        }

        #[test]
        fn test_patterns_with_spaces() {
            let re_str = r"hel+o world";
            let search_config = SearchConfig {
                search_text: re_str,
                fixed_strings: false,
                match_whole_word: true,
                match_case: false,
                replacement_text: "hi earth",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("say hello world!", &parsed.search, &parsed.replace),
                Some("say hi earth!".to_string())
            );
            let search_config = SearchConfig {
                search_text: re_str,
                fixed_strings: false,
                match_whole_word: true,
                match_case: false,
                replacement_text: "hi earth",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("helloworld", &parsed.search, &parsed.replace),
                None
            );
        }

        #[test]
        fn test_multiple_matches() {
            let re_str = r"a+b+";
            let search_config = SearchConfig {
                search_text: re_str,
                fixed_strings: false,
                match_whole_word: true,
                match_case: false,
                replacement_text: "X",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("foo aab abb", &parsed.search, &parsed.replace),
                Some("foo X X".to_string())
            );
            let search_config = SearchConfig {
                search_text: re_str,
                fixed_strings: false,
                match_whole_word: true,
                match_case: false,
                replacement_text: "X",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("ab abaab abb", &parsed.search, &parsed.replace),
                Some("X abaab X".to_string())
            );
            let search_config = SearchConfig {
                search_text: re_str,
                fixed_strings: false,
                match_whole_word: true,
                match_case: false,
                replacement_text: "X",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("ababaababb", &parsed.search, &parsed.replace),
                None
            );
            let search_config = SearchConfig {
                search_text: re_str,
                fixed_strings: false,
                match_whole_word: true,
                match_case: false,
                replacement_text: "X",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("ab ab aab abb", &parsed.search, &parsed.replace),
                Some("X X X X".to_string())
            );
        }

        #[test]
        fn test_boundary_cases() {
            let re_str = r"foo\s*bar";
            // At start of string
            let search_config = SearchConfig {
                search_text: re_str,
                fixed_strings: false,
                match_whole_word: true,
                match_case: false,
                replacement_text: "TEST",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("foo bar baz", &parsed.search, &parsed.replace),
                Some("TEST baz".to_string())
            );
            // At end of string
            let search_config = SearchConfig {
                search_text: re_str,
                fixed_strings: false,
                match_whole_word: true,
                match_case: false,
                replacement_text: "TEST",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("baz foo bar", &parsed.search, &parsed.replace),
                Some("baz TEST".to_string())
            );
            // With punctuation
            let search_config = SearchConfig {
                search_text: re_str,
                fixed_strings: false,
                match_whole_word: true,
                match_case: false,
                replacement_text: "TEST",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("a (?( foo  bar)", &parsed.search, &parsed.replace),
                Some("a (?( TEST)".to_string())
            );
        }

        #[test]
        fn test_with_punctuation() {
            let re_str = r"a\d+b";
            let search_config = SearchConfig {
                search_text: re_str,
                fixed_strings: false,
                match_whole_word: true,
                match_case: false,
                replacement_text: "X",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("(a42b)", &parsed.search, &parsed.replace),
                Some("(X)".to_string())
            );
            let search_config = SearchConfig {
                search_text: re_str,
                fixed_strings: false,
                match_whole_word: true,
                match_case: false,
                replacement_text: "X",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("foo.a123b!bar", &parsed.search, &parsed.replace),
                Some("foo.X!bar".to_string())
            );
        }

        #[test]
        fn test_complex_patterns() {
            let re_str = r"[a-z]+\d+[a-z]+";
            let search_config = SearchConfig {
                search_text: re_str,
                fixed_strings: false,
                match_whole_word: true,
                match_case: false,
                replacement_text: "NEW",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("test9 abc123def 8xyz", &parsed.search, &parsed.replace),
                Some("test9 NEW 8xyz".to_string())
            );
            let search_config = SearchConfig {
                search_text: re_str,
                fixed_strings: false,
                match_whole_word: true,
                match_case: false,
                replacement_text: "NEW",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("test9abc123def8xyz", &parsed.search, &parsed.replace),
                None
            );
        }

        #[test]
        fn test_optional_patterns() {
            let re_str = r"colou?r";
            let search_config = SearchConfig {
                search_text: re_str,
                fixed_strings: false,
                match_whole_word: true,
                match_case: false,
                replacement_text: "X",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("my color and colour", &parsed.search, &parsed.replace),
                Some("my X and X".to_string())
            );
        }

        #[test]
        fn test_empty_haystack() {
            let re_str = r"test";
            let search_config = SearchConfig {
                search_text: re_str,
                fixed_strings: false,
                match_whole_word: true,
                match_case: false,
                replacement_text: "NEW",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("", &parsed.search, &parsed.replace),
                None
            );
        }

        #[test]
        fn test_empty_search_regex() {
            let re_str = r"";
            let search_config = SearchConfig {
                search_text: re_str,
                fixed_strings: false,
                match_whole_word: true,
                match_case: false,
                replacement_text: "NEW",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("search", &parsed.search, &parsed.replace),
                None
            );
        }

        #[test]
        fn test_single_char() {
            let re_str = r"a";
            let search_config = SearchConfig {
                search_text: re_str,
                fixed_strings: false,
                match_whole_word: true,
                match_case: false,
                replacement_text: "X",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("b a c", &parsed.search, &parsed.replace),
                Some("b X c".to_string())
            );
            let search_config = SearchConfig {
                search_text: re_str,
                fixed_strings: false,
                match_whole_word: true,
                match_case: false,
                replacement_text: "X",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("bac", &parsed.search, &parsed.replace),
                None
            );
        }

        #[test]
        fn test_escaped_chars() {
            let re_str = r"\(\d+\)";
            let search_config = SearchConfig {
                search_text: re_str,
                fixed_strings: false,
                match_whole_word: true,
                match_case: false,
                replacement_text: "X",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("test (123) foo", &parsed.search, &parsed.replace),
                Some("test X foo".to_string())
            );
        }

        #[test]
        fn test_with_unicode() {
            let re_str = r"λ\d+";
            let search_config = SearchConfig {
                search_text: re_str,
                fixed_strings: false,
                match_whole_word: true,
                match_case: false,
                replacement_text: "X",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("calc λ123 β", &parsed.search, &parsed.replace),
                Some("calc X β".to_string())
            );
            let search_config = SearchConfig {
                search_text: re_str,
                fixed_strings: false,
                match_whole_word: true,
                match_case: false,
                replacement_text: "X",
                advanced_regex: false,
            };
            let parsed = test_helpers::must_parse_search_config(search_config);
            assert_eq!(
                replacement_if_match("calcλ123", &parsed.search, &parsed.replace),
                None
            );
        }
    }
}
