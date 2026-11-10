use std::fmt::Write;
use td_link::net::wire::is_format;

pub(crate) fn shown(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '\u{2028}' | '\u{2029}' => escape(&mut out, c),
            c if c.is_control() || is_format(c) => escape(&mut out, c),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
fn escape(out: &mut String, c: char) {
    let _ = write!(out, "\\u{{{:04X}}}", u32::from(c));
}
