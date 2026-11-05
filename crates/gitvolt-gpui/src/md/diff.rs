//! The rendered diff (`ui/src/markdown/diff/`).

#[derive(Default)]
pub struct CodeLines {
    pub value: String,
    pub marks: String,
    pub words: Option<String>,
}

pub fn code_lines(_old: &str, _new: &str, _deadline: Option<std::time::Instant>) -> Option<CodeLines> {
    None
}
