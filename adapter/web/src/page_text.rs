//! The visible text of a page as lines, for monitoring pages that state no structured product
//! data. Script, style, template, noscript, svg and head content is not visible and is skipped;
//! block elements end a line; whitespace inside a line collapses to single spaces.

use ecdev_core::sha256::Sha256;
use scraper::{ElementRef, Html};

const HIDDEN: [&str; 7] = [
    "script", "style", "template", "noscript", "svg", "head", "iframe",
];
const BLOCK: [&str; 31] = [
    "address",
    "article",
    "aside",
    "blockquote",
    "br",
    "dd",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "li",
    "main",
    "nav",
    "p",
    "section",
    "table",
    "tr",
    "td",
    "th",
    "pre",
];

/// Not rendered by the browser as written in the markup: the `hidden` attribute, or an inline
/// style of display:none or visibility:hidden. Stylesheet rules are not evaluated, and
/// aria-hidden is left alone (it hides from assistive technology, not from sight).
fn hidden_by_markup(e: ElementRef) -> bool {
    e.value().attr("hidden").is_some()
        || e.value().attr("style").is_some_and(|style| {
            style.split(';').any(|decl| {
                let mut parts = decl.splitn(2, ':');
                let (name, value) = (
                    parts.next().unwrap_or("").trim().to_ascii_lowercase(),
                    parts
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_ascii_lowercase()
                        .replace("!important", "")
                        .trim()
                        .to_string(),
                );
                (name == "display" && value == "none")
                    || (name == "visibility" && value == "hidden")
            })
        })
}

fn flush(current: &mut String, lines: &mut Vec<String>) {
    let line = current.split_whitespace().collect::<Vec<_>>().join(" ");
    if !line.is_empty() {
        lines.push(line);
    }
    current.clear();
}

fn walk(element: ElementRef, current: &mut String, lines: &mut Vec<String>, depth: usize) {
    let name = element.value().name();
    if HIDDEN.contains(&name) || depth > 512 || hidden_by_markup(element) {
        return;
    }
    let block = BLOCK.contains(&name);
    if block {
        flush(current, lines);
    }
    for child in element.children() {
        if let Some(e) = ElementRef::wrap(child) {
            walk(e, current, lines, depth + 1);
        } else if let Some(t) = child.value().as_text() {
            current.push(' ');
            current.push_str(t);
        }
    }
    if block {
        flush(current, lines);
    }
}

/// Landmarks that frame a page rather than state its content: site navigation, banners, footers
/// and side panels, by element or by ARIA role. Store pages change these often (cart counts,
/// promotion strips) without changing what the page says about its product or terms.
const CHROME: [&str; 4] = ["nav", "header", "footer", "aside"];
const CHROME_ROLES: [&str; 5] = [
    "navigation",
    "banner",
    "contentinfo",
    "complementary",
    "search",
];

/// Inside main, article or section, a header or footer belongs to that section (its heading,
/// its byline), not to the page frame.
fn chrome(e: ElementRef, sectioned: bool) -> bool {
    let name = e.value().name();
    (CHROME.contains(&name) && !(sectioned && matches!(name, "header" | "footer")))
        || e.value()
            .attr("role")
            .is_some_and(|r| CHROME_ROLES.contains(&r.trim().to_ascii_lowercase().as_str()))
}

fn main_walk(
    element: ElementRef,
    current: &mut String,
    lines: &mut Vec<String>,
    depth: usize,
    sectioned: bool,
) {
    let name = element.value().name();
    if HIDDEN.contains(&name)
        || chrome(element, sectioned)
        || depth > 512
        || hidden_by_markup(element)
    {
        return;
    }
    let sectioned = sectioned
        || matches!(name, "main" | "article" | "section")
        || element.value().attr("role") == Some("main");
    let block = BLOCK.contains(&name);
    if block {
        flush(current, lines);
    }
    for child in element.children() {
        if let Some(e) = ElementRef::wrap(child) {
            main_walk(e, current, lines, depth + 1, sectioned);
        } else if let Some(t) = child.value().as_text() {
            current.push(' ');
            current.push_str(t);
        }
    }
    if block {
        flush(current, lines);
    }
}

/// The page's main content as lines: inside its <main> (or role="main") landmark when it has
/// one, otherwise the whole page; landmark chrome (nav, header, footer, aside and their roles)
/// left out either way. Returns the lines and which scope they come from.
pub fn main_lines(html: &str) -> (Vec<String>, &'static str) {
    let doc = Html::parse_document(html);
    let main = scraper::Selector::parse("main, [role=main]").unwrap();
    let (root, scope) = match doc.select(&main).next() {
        Some(m) => (m, "MAIN_LANDMARK"),
        None => (doc.root_element(), "PAGE_WITHOUT_CHROME_LANDMARKS"),
    };
    let mut lines = vec![];
    let mut current = String::new();
    main_walk(root, &mut current, &mut lines, 0, false);
    flush(&mut current, &mut lines);
    (lines, scope)
}

pub fn visible_lines(html: &str) -> Vec<String> {
    let doc = Html::parse_document(html);
    let mut lines = vec![];
    let mut current = String::new();
    walk(doc.root_element(), &mut current, &mut lines, 0);
    flush(&mut current, &mut lines);
    lines
}

/// Lines kept with a capture's text fingerprint, so changes can be shown, not just detected.
pub const MAX_KEPT_LINES: usize = 1_000;
pub const MAX_KEPT_BYTES: usize = 100 * 1024;

/// The fingerprint of the whole visible text, with its first lines up to the bounds.
pub fn summary(html: &str) -> serde_json::Value {
    let lines = visible_lines(html);
    let mut kept = vec![];
    let mut bytes = 0;
    for l in lines.iter().take(MAX_KEPT_LINES) {
        bytes += l.len() + 1;
        if bytes > MAX_KEPT_BYTES {
            break;
        }
        kept.push(l.clone());
    }
    let (main, scope) = main_lines(html);
    serde_json::json!({"sha256":text_sha256(&lines),"line_count":lines.len(),"lines":kept,"truncated":kept.len() < lines.len(),"method":"VISIBLE_TEXT_LINES_V1",
        "main_sha256":text_sha256(&main),"main_line_count":main.len(),"main_scope":scope,"substance":substance(&main)})
}

/// How much readable text a page's main content holds, as measured numbers, not a verdict:
/// characters, and a score summing the square root of each line's length beyond `LONG_LINE`
/// characters (long runs of prose count, menus and buttons do not, and one very long line
/// counts for little). A page that is a JavaScript app shell scores near zero, and a text watch
/// on it can only ever see the shell.
pub const LONG_LINE: usize = 140;

pub fn substance(main: &[String]) -> serde_json::Value {
    let chars: usize = main.iter().map(|l| l.chars().count()).sum();
    let score: f64 = main
        .iter()
        .map(|l| l.chars().count())
        .filter(|n| *n > LONG_LINE)
        .map(|n| ((n - LONG_LINE) as f64).sqrt())
        .sum::<f64>()
        // An empty float sum is -0.0.
        + 0.0;
    serde_json::json!({"main_chars":chars,"main_long_lines":main.iter().filter(|l| l.chars().count() > LONG_LINE).count(),"long_line_score":score,"long_line_threshold_chars":LONG_LINE,"basis":"SUM_SQRT_LINE_LENGTH_BEYOND_THRESHOLD_OF_MAIN_CONTENT"})
}

pub fn text_sha256(lines: &[String]) -> String {
    format!("{:x}", Sha256::digest(lines.join("\n").as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_visible_text_in_lines() {
        let html = r#"<html><head><title>T</title><style>p{}</style></head><body>
            <script>var x = Date.now();</script>
            <h1>Shipping   policy</h1><p>Orders ship in <b>2</b> days.</p>
            <ul><li>Free over 50</li><li>Returns 30 days</li></ul>
            <noscript>enable js</noscript><template><p>hidden</p></template>
            <div>Last <span>updated</span></div></body></html>"#;
        assert_eq!(
            visible_lines(html),
            [
                "Shipping policy",
                "Orders ship in 2 days.",
                "Free over 50",
                "Returns 30 days",
                "Last updated"
            ]
        );
        assert_eq!(
            text_sha256(&visible_lines(html)),
            text_sha256(&visible_lines(&html.replace("Date.now()", "0")))
        );
    }

    #[test]
    fn text_hidden_by_its_own_markup_is_not_page_text() {
        let html = r#"<body><p>Price 1200</p><div hidden>Old price 1500</div>
            <p style="color:red; Display : NONE !important">promo</p><span style="visibility:hidden">ghost</span>
            <p style="display:block">Shown</p><p aria-hidden="true">Icon label</p></body>"#;
        assert_eq!(visible_lines(html), ["Price 1200", "Shown", "Icon label"]);
        assert_eq!(main_lines(html).0, ["Price 1200", "Shown", "Icon label"]);
    }

    #[test]
    fn substance_counts_prose_not_chrome_or_shells() {
        let prose = "x".repeat(140 + 100);
        let s = substance(&["Shop".to_string(), prose.clone(), prose]);
        assert_eq!(s["main_long_lines"], 2);
        assert_eq!(s["long_line_score"], 20.0);
        assert_eq!(substance(&["Loading".to_string()])["long_line_score"], 0.0);
        assert!(
            substance(&[])["long_line_score"]
                .as_f64()
                .unwrap()
                .is_sign_positive()
        );
        let shell = summary("<html><body><div id=app></div><script>boot()</script></body></html>");
        assert_eq!(shell["substance"]["main_chars"], 0);
    }

    #[test]
    fn main_content_leaves_out_page_chrome() {
        let page = |cart: &str, price: &str| {
            format!(
                r#"<html><body><header><a>Cart ({cart})</a></header>
            <nav><a>Tea</a></nav><div role="banner">Sale ends soon</div>
            <main><h1>Matcha whisk</h1><p>Price {price}</p><aside>Related</aside></main>
            <footer>© shop</footer></body></html>"#
            )
        };
        let (lines, scope) = main_lines(&page("1", "1200"));
        assert_eq!(scope, "MAIN_LANDMARK");
        assert_eq!(lines, ["Matcha whisk", "Price 1200"]);
        let (a, b, c) = (
            summary(&page("1", "1200")),
            summary(&page("2", "1200")),
            summary(&page("1", "1300")),
        );
        assert_ne!(a["sha256"], b["sha256"], "the whole page changed");
        assert_eq!(a["main_sha256"], b["main_sha256"], "only its chrome did");
        assert_ne!(a["main_sha256"], c["main_sha256"]);
        let (article, _) = main_lines(
            "<body><header>Shop</header><main><article><header><h1>Our story</h1></header><p>Body</p><footer>By the team</footer></article></main></body>",
        );
        assert_eq!(article, ["Our story", "Body", "By the team"]);
        let (bare, scope) = main_lines(
            "<body><nav>Menu</nav><p>Terms text</p><div role=contentinfo>x</div></body>",
        );
        assert_eq!(
            (bare, scope),
            (
                vec!["Terms text".to_string()],
                "PAGE_WITHOUT_CHROME_LANDMARKS"
            )
        );
    }
}
