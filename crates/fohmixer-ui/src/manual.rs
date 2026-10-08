//! The tag manual (#68, spec D16): one page, `znacky.html`, served by the
//! hub (the tray opens it) and shown by the column's ZNAČKY chip in an
//! overlay. The hub sends every page with `X-Frame-Options: DENY`, so the
//! overlay fetches the file and shows its `<main>` with its `<style>`.

/// The style and the `<main>` element of the manual page, when it has both.
pub fn manual_parts(html: &str) -> Option<(String, String)> {
    let style = between(html, "<style>", "</style>")?;
    let start = html.find("<main")?;
    let end = html[start..].find("</main>")? + start + "</main>".len();
    Some((style.to_string(), html[start..end].to_string()))
}

/// The text between the first `open` and the `close` after it.
fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let from = text.find(open)? + open.len();
    let to = text[from..].find(close)? + from;
    Some(&text[from..to])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_style_and_the_main_element_are_taken_out() {
        let html = "<html><head><style>.z{a:b}</style></head><body><main class=\"z\">x</main> y</body></html>";
        assert_eq!(
            manual_parts(html),
            Some((
                ".z{a:b}".to_string(),
                "<main class=\"z\">x</main>".to_string()
            ))
        );
        assert_eq!(manual_parts("<style>a</style>"), None);
        assert_eq!(manual_parts("<main>x</main>"), None);
        assert_eq!(manual_parts("<style>a</style><main>x"), None);
        assert_eq!(manual_parts("<style>a<main>x</main>"), None);
    }

    #[test]
    fn the_shipped_manual_has_both() {
        let (style, main) = manual_parts(include_str!("../znacky.html")).expect("both parts");
        assert!(style.contains(".znacky"), "{style}");
        assert!(main.starts_with("<main class=\"znacky\""), "{main}");
        assert!(main.ends_with("</main>"), "{main}");
        assert!(main.contains("+G:VOCALS:2"), "the syntax");
        assert!(main.contains("KONFLIKT"), "the problems");
    }
}
