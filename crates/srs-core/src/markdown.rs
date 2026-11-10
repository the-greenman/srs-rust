//! Pure markdown → HTML rendering (srs-rust#1191).
//!
//! The output is safe to inject with `innerHTML` / `{@html}`: raw HTML in the
//! source is emitted as escaped text, and link/image URLs are limited to
//! `http`, `https`, `mailto` and scheme-less (relative) URLs.
//!
//! [`render_markdown_with`] can render without images (srs-rust#1606): each
//! image becomes its alt text, as escaped plain text, and no `<img>` is emitted.

use pulldown_cmark::{html, CowStr, Event, Options, Parser, Tag, TagEnd};

/// True when `url` is http(s), mailto, or has no scheme (relative).
fn url_allowed(url: &str) -> bool {
    // Browsers ignore ASCII whitespace/control chars inside a scheme
    // ("java\tscript:"), so strip them before looking for one.
    let u: String = url
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_control())
        .collect::<String>()
        .to_ascii_lowercase();
    match u.find([':', '/', '?', '#']) {
        Some(i) if u.as_bytes()[i] == b':' => {
            matches!(&u[..i], "http" | "https" | "mailto")
        }
        _ => true, // no scheme: relative
    }
}

fn sanitize(event: Event<'_>) -> Event<'_> {
    match event {
        // ponytail: escaped as text, so `<b>` shows literally; no allowlisted-tag pass-through.
        Event::Html(s) | Event::InlineHtml(s) => Event::Text(s),
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) if !url_allowed(&dest_url) => Event::Start(Tag::Link {
            link_type,
            dest_url: CowStr::Borrowed(""),
            title,
            id,
        }),
        Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) if !url_allowed(&dest_url) => Event::Start(Tag::Image {
            link_type,
            dest_url: CowStr::Borrowed(""),
            title,
            id,
        }),
        e => e,
    }
}

/// Options for [`render_markdown_with`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderOptions {
    /// Emit `<img>` for images. When false, an image renders as its alt text.
    pub images: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self { images: true }
    }
}

/// Render CommonMark (plus tables and strikethrough) to safe HTML.
pub fn render_markdown(md: &str) -> String {
    render_markdown_with(md, RenderOptions::default())
}

/// [`render_markdown`] with options; the default options give the same output.
pub fn render_markdown_with(md: &str, options: RenderOptions) -> String {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
    let events = Parser::new_ext(md, opts).map(sanitize);
    let mut out = String::new();
    if options.images {
        html::push_html(&mut out, events);
    } else {
        // Inside an image (depth > 0) keep only the alt text, flattened to
        // plain text the way pulldown-cmark writes an `alt` attribute.
        let mut depth = 0usize;
        html::push_html(
            &mut out,
            events.filter_map(move |e| match e {
                Event::Start(Tag::Image { .. }) => {
                    depth += 1;
                    None
                }
                Event::End(TagEnd::Image) => {
                    depth -= 1;
                    None
                }
                e if depth == 0 => Some(e),
                Event::Text(s) | Event::Code(s) => Some(Event::Text(s)),
                Event::SoftBreak | Event::HardBreak => Some(Event::Text(CowStr::Borrowed(" "))),
                _ => None,
            }),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::render_markdown as r;

    #[test]
    fn headings_emphasis() {
        assert_eq!(
            r("# T\n\n*a* **b** ~~c~~ `d`"),
            "<h1>T</h1>\n<p><em>a</em> <strong>b</strong> <del>c</del> <code>d</code></p>\n"
        );
    }

    #[test]
    fn lists() {
        assert_eq!(
            r("- a\n- b\n\n1. x\n2. y"),
            "<ul>\n<li>a</li>\n<li>b</li>\n</ul>\n<ol>\n<li>x</li>\n<li>y</li>\n</ol>\n"
        );
    }

    #[test]
    fn tables() {
        assert_eq!(
            r("|a|b|\n|-|-|\n|1|2|"),
            "<table><thead><tr><th>a</th><th>b</th></tr></thead><tbody>\n<tr><td>1</td><td>2</td></tr>\n</tbody></table>\n"
        );
    }

    #[test]
    fn code_block() {
        assert_eq!(
            r("```rust\nlet a = 1 < 2;\n```"),
            "<pre><code class=\"language-rust\">let a = 1 &lt; 2;\n</code></pre>\n"
        );
    }

    #[test]
    fn safe_links() {
        assert_eq!(
            r("[x](https://e.com/a?b=1) [m](mailto:a@b.c) [r](../up#h) [q](a/b:c)"),
            "<p><a href=\"https://e.com/a?b=1\">x</a> <a href=\"mailto:a@b.c\">m</a> <a href=\"../up#h\">r</a> <a href=\"a/b:c\">q</a></p>\n"
        );
    }

    #[test]
    fn xss_raw_html_escaped() {
        assert_eq!(
            r("<script>alert(1)</script>"),
            "&lt;script&gt;alert(1)&lt;/script&gt;"
        );
        let img = r("<img src=x onerror=alert(1)>");
        assert!(!img.contains("<img"), "{img}");
        let a = r("hi <a href=\"javascript:alert(1)\">x</a>");
        assert!(!a.contains("<a"), "{a}");
    }

    #[test]
    fn xss_bad_schemes_dropped() {
        for md in [
            "[x](javascript:alert(1))",
            "[x]( JaVaScRiPt:alert(1))",
            "[x](<java\tscript:alert(1)>)",
            "[x](vbscript:x)",
            "[x](data:text/html,<script>1</script>)",
            "![x](data:text/html,<script>alert(1)</script>)",
            "[x]: javascript:alert(1)\n\n[x]",
        ] {
            let h = r(md);
            assert!(!h.to_lowercase().contains("script:"), "{md} -> {h}");
            assert!(!h.contains("data:"), "{md} -> {h}");
            assert!(!h.contains("<script"), "{md} -> {h}");
        }
        assert_eq!(r("[x](javascript:alert(1))"), "<p><a href=\"\">x</a></p>\n");
        assert_eq!(
            r("![x](data:text/html,a)"),
            "<p><img src=\"\" alt=\"x\" /></p>\n"
        );
    }

    fn no_img(md: &str) -> String {
        super::render_markdown_with(md, super::RenderOptions { images: false })
    }

    #[test]
    fn images_off_render_alt_text() {
        assert_eq!(
            no_img("a ![cat <x>](c.png) b"),
            "<p>a cat &lt;x&gt; b</p>\n"
        );
        assert_eq!(
            no_img("[![logo](l.png)](https://e.com)"),
            "<p><a href=\"https://e.com\">logo</a></p>\n"
        );
        assert_eq!(no_img("![ref][r]\n\n[r]: r.png"), "<p>ref</p>\n");
        assert_eq!(no_img("![*a* **b** `c`](x.png)"), "<p>a b c</p>\n");
        assert_eq!(no_img("![x](data:text/html,a)"), "<p>x</p>\n");
        for md in [
            "![a](https://e.com/a.png)",
            "![](a.png)",
            "![a ![b](b.png)](a.png)",
        ] {
            assert!(!no_img(md).contains("<img"), "{md}");
        }
    }

    #[test]
    fn images_default_unchanged() {
        for md in [
            "a ![cat](c.png \"t\") b",
            "![*a*](x.png)",
            "<b>x</b> [l](u)",
        ] {
            assert_eq!(super::render_markdown_with(md, Default::default()), r(md));
        }
        assert_eq!(r("![c](c.png)"), "<p><img src=\"c.png\" alt=\"c\" /></p>\n");
    }
}
