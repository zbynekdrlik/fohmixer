//! The tag manual (#68, spec D16): one page, `znacky.html`, served by the
//! hub (the tray opens it) and shown by the column's ZNAČKY chip in an
//! overlay. The hub sends every page with `X-Frame-Options: DENY`, so the
//! overlay fetches the file and shows its `<main>` with its `<style>`.

/// The style of the head and the `<main>` element of the body of the manual
/// page, when it has both (a comment before the head never counts).
pub fn manual_parts(html: &str) -> Option<(String, String)> {
    let head = &html[html.find("<head>")?..];
    let style = between(head, "<style>", "</style>")?;
    let body = &html[html.find("<body")?..];
    let start = body.find("<main")?;
    let end = body[start..].find("</main>")? + start + "</main>".len();
    Some((style.to_string(), body[start..end].to_string()))
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
        let none = [
            "<head></head><body><main>x</main></body>",
            "<head><style>a</style></head><body></body>",
            "<head><style>a</style></head><body><main>x",
            "<head><style>a<body><main>x</main>",
            "<style>a</style><body><main>x</main>",
            "<head><style>a</style></head><main>x</main>",
        ];
        for html in none {
            assert_eq!(manual_parts(html), None, "{html}");
        }
    }

    #[test]
    fn a_comment_before_the_head_naming_both_is_left_out() {
        let html = "<!-- its <main> with this <style> --><head><style>s</style></head><body><main>m</main></body>";
        assert_eq!(
            manual_parts(html),
            Some(("s".to_string(), "<main>m</main>".to_string()))
        );
    }

    #[test]
    fn the_shipped_manual_has_both() {
        let (style, main) = manual_parts(include_str!("../znacky.html")).expect("both parts");
        assert!(style.trim_start().starts_with(".znacky {"), "{style}");
        assert!(main.starts_with("<main class=\"znacky\""), "{main}");
        assert_eq!(main.matches("<main").count(), 1, "{main}");
        assert!(main.ends_with("</main>"), "{main}");
        assert!(main.contains("+G:VOCALS:2"), "the syntax");
        assert!(main.contains("KONFLIKT"), "the problems");
    }
}
