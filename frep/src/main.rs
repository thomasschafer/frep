use std::str::FromStr;

use clap::Parser;
use simple_log::LevelFilter;

mod logging;

#[derive(Parser, Debug)]
#[command(about = "Find and replace CLI.")]
#[command(version)]
#[allow(clippy::struct_excessive_bools)]
struct Args {
    /// Text to search with. This will be regex, unless --fixed-strings is used in which case this is a string literal
    #[arg(index = 1)]
    search_text: String,

    /// Text to replace the search text with. If blank then the search text will be deleted. This can include capture groups if using search regex
    #[arg(index = 2)]
    replace_text: Option<String>,

    /// Directory in which to search
    #[arg(short, long, default_value = ".")]
    directory: String,

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
    files_to_include: Option<String>,

    /// Glob patterns, separated by commas (,), that file paths must not match
    #[arg(short = 'E', long)]
    files_to_exclude: Option<String>,

    /// Include hidden files and directories, such as those whose name starts with a dot (.)
    #[arg(short = '.', long, default_value = "false")]
    hidden: bool,

    /// Log level (trace, debug, info, warn, error)
    #[arg(
        long,
        value_parser = parse_log_level,
        default_value = logging::DEFAULT_LOG_LEVEL
    )]
    log_level: LevelFilter,

    /// Use advanced regex features (including negative look-ahead), at the cost of performance
    #[arg(short = 'a', long, default_value = "false")]
    advanced_regex: bool,
}

fn parse_log_level(s: &str) -> Result<LevelFilter, String> {
    LevelFilter::from_str(s).map_err(|_| format!("Invalid log level: {s}"))
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    logging::setup_logging(args.log_level)?;

    todo!()
}
