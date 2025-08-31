use anyhow::bail;
use clap::Parser;
use frep_core::validation::{DirConfig, SearchConfig};
use simple_log::LevelFilter;
use std::{
    io::{self, IsTerminal, Read},
    path::PathBuf,
    str::FromStr,
};

use frep_core::run;

mod logging;

#[derive(Parser, Debug)]
#[command(about = "Find and replace CLI. Processes files in a directory or text from stdin.")]
#[command(version)]
#[allow(clippy::struct_excessive_bools)]
struct Args {
    /// Text to search with. This will be regex, unless --fixed-strings is used in which case this is a string literal
    #[arg(index = 1)]
    search_text: String,

    /// Text to replace the search text with. This can include capture groups if using search regex. If left blank (and --delete is used) then the search text will be deleted
    #[arg(index = 2)]
    replace_text: Option<String>,

    /// Directory in which to search
    #[arg(short, long, value_parser = parse_directory, default_value = ".")]
    directory: PathBuf,

    /// Search with plain strings, rather than regex
    #[arg(short, long, action = clap::ArgAction::SetTrue)]
    fixed_strings: bool,

    /// Only match when the search string forms an entire word, and not a substring in a larger word
    #[arg(short = 'w', long, action = clap::ArgAction::SetTrue)]
    match_whole_word: bool,

    /// Ignore case when matching the search string
    #[arg(short = 'i', long, action = clap::ArgAction::SetTrue)]
    case_insensitive: bool,

    /// Glob patterns, separated by commas (,), that file paths must match
    #[arg(short = 'I', long)]
    include_files: Option<String>,

    /// Glob patterns, separated by commas (,), that file paths must not match
    #[arg(short = 'E', long)]
    exclude_files: Option<String>,

    /// Include hidden files and directories, such as those whose name starts with a dot (.)
    #[arg(short = '.', long, action = clap::ArgAction::SetTrue)]
    hidden: bool,

    /// Log level (trace, debug, info, warn, error)
    #[arg(
        long,
        value_parser = parse_log_level,
        default_value = logging::DEFAULT_LOG_LEVEL
    )]
    log_level: LevelFilter,

    /// Use advanced regex features (including negative look-ahead), at the cost of performance
    #[arg(short = 'a', long, action = clap::ArgAction::SetTrue)]
    advanced_regex: bool,

    /// Delete matches
    #[arg(short = 'D', long, action = clap::ArgAction::SetTrue)]
    delete: bool,
}

fn detect_and_read_stdin() -> anyhow::Result<Option<String>> {
    if io::stdin().is_terminal() {
        return Ok(None);
    }

    let mut stdin_content = String::new();
    io::stdin().read_to_string(&mut stdin_content)?;

    if stdin_content.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(stdin_content))
    }
}

fn validate_args(args: &Args, stdin_content: Option<&String>) -> anyhow::Result<()> {
    if args.search_text.is_empty() {
        bail!("Search text must not be empty");
    }

    if args.replace_text.is_none() && !args.delete {
        bail!(
            "You must either specify either replacement text (`frep before after`) or use --delete to delete matches `(frep before --delete)`"
        );
    }
    if args.replace_text.is_some() && args.delete {
        bail!(
            "You cannot specify both replacement text and the --delete flag. Use either replacement text (`frep before after`) or the --delete flag (`frep before --delete`)"
        );
    }

    if stdin_content.is_some() {
        if args.hidden {
            bail!("Cannot use --hidden flag with stdin input");
        }
        if args.include_files.is_some() {
            bail!("Cannot use --include-files with stdin input");
        }
        if args.exclude_files.is_some() {
            bail!("Cannot use --exclude-files with stdin input");
        }
    }

    Ok(())
}

fn parse_log_level(level: &str) -> Result<LevelFilter, String> {
    LevelFilter::from_str(level).map_err(|_| format!("Invalid log level: {level}"))
}

fn parse_directory(dir: &str) -> anyhow::Result<PathBuf> {
    let path = PathBuf::from(dir);
    if path.exists() {
        Ok(path)
    } else {
        bail!("'{dir}' does not exist. Please provide a valid path.")
    }
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let stdin_content = detect_and_read_stdin()?;

    validate_args(&args, stdin_content.as_ref())?;
    logging::setup_logging(args.log_level)?;

    let search_config = search_config_from_args(&args);
    let results = if let Some(stdin_content) = stdin_content {
        run::find_and_replace_text(&stdin_content, search_config)?
    } else {
        run::find_and_replace(search_config, dir_config_from_args(&args))?
    };

    println!("{results}");
    Ok(())
}

fn dir_config_from_args(args: &Args) -> DirConfig<'_> {
    let dir_config = DirConfig {
        include_globs: args.include_files.as_deref(),
        exclude_globs: args.exclude_files.as_deref(),
        include_hidden: args.hidden,
        directory: args.directory.clone(),
    };
    dir_config
}

fn search_config_from_args(args: &Args) -> SearchConfig<'_> {
    let search_config = SearchConfig {
        search_text: &args.search_text,
        replacement_text: args.replace_text.as_deref().unwrap_or(""),
        fixed_strings: args.fixed_strings,
        advanced_regex: args.advanced_regex,
        match_whole_word: args.match_whole_word,
        match_case: !args.case_insensitive,
    };
    search_config
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_test_dir() -> TempDir {
        TempDir::new().unwrap()
    }

    #[test]
    fn test_validate_directory_exists() {
        let temp_dir = setup_test_dir();
        let dir_path = temp_dir.path().to_str().unwrap();

        let result = parse_directory(dir_path);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), PathBuf::from(dir_path));
    }

    #[test]
    fn test_validate_directory_does_not_exist() {
        let nonexistent_path = "/path/that/definitely/does/not/exist/12345";
        let result = parse_directory(nonexistent_path);

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("does not exist"));
        assert!(err.contains(nonexistent_path));
    }

    #[test]
    fn test_validate_directory_with_nested_structure() {
        let temp_dir = setup_test_dir();
        let nested_dir = temp_dir.path().join("nested").join("directory");
        std::fs::create_dir_all(&nested_dir).expect("Failed to create nested directories");

        let dir_path = nested_dir.to_str().unwrap();
        let result = parse_directory(dir_path);

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), nested_dir);
    }

    #[test]
    fn test_validate_directory_with_special_chars() {
        let temp_dir = setup_test_dir();
        let special_dir = temp_dir.path().join("test with spaces and-symbols_!@#$");
        std::fs::create_dir(&special_dir)
            .expect("Failed to create directory with special characters");

        let dir_path = special_dir.to_str().unwrap();
        let result = parse_directory(dir_path);

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), special_dir);
    }

    fn test_args() -> Args {
        Args {
            search_text: "search".to_string(),
            replace_text: Some("replace".to_string()),
            directory: PathBuf::from("."),
            fixed_strings: false,
            match_whole_word: false,
            case_insensitive: false,
            include_files: None,
            exclude_files: None,
            hidden: false,
            log_level: LevelFilter::Info,
            advanced_regex: false,
            delete: false,
        }
    }

    #[test]
    fn test_validate_args_with_replacement_text() {
        let args = Args {
            replace_text: Some("replace".to_string()),
            delete: false,
            ..test_args()
        };

        let result = validate_args(&args, None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_args_with_delete_flag() {
        let args = Args {
            replace_text: None,
            delete: true,
            ..test_args()
        };

        let result = validate_args(&args, None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_args_with_both_replacement_and_delete() {
        let args = Args {
            replace_text: Some("replace".to_string()),
            delete: true,
            ..test_args()
        };

        let result = validate_args(&args, None);
        assert!(result.is_err());

        let error_message = result.unwrap_err().to_string();
        assert!(
            error_message.contains("cannot specify both")
                && error_message.contains("replacement text")
                && error_message.contains("--delete"),
            "Error message should explain that both options cannot be used together"
        );
    }

    #[test]
    fn test_validate_args_with_neither_replacement_nor_delete() {
        let args = Args {
            replace_text: None,
            delete: false,
            ..test_args()
        };

        let result = validate_args(&args, None);
        assert!(result.is_err());

        let error_message = result.unwrap_err().to_string();
        assert!(
            error_message.contains("must either specify")
                && error_message.contains("replacement text")
                && error_message.contains("--delete"),
            "Error message should mention both replacement text and --delete option"
        );
    }
}
