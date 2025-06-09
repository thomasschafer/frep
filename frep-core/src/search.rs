use std::path::PathBuf;
use crate::line_reader::LineEnding;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchResult {
    pub path: PathBuf,
    /// 1-indexed
    pub line_number: usize,
    pub line: String,
    pub line_ending: LineEnding,
    pub replacement: String,
    pub included: bool,
    pub replace_result: Option<ReplaceResult>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplaceResult {
    /// The replacement was successful
    Success,
    /// The replacement was not successful because of an error
    Error(String),
}
