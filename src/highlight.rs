use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};
use std::sync::LazyLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::{SyntaxDefinition, SyntaxSet};
use syntect::util::LinesWithEndings;

static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(|| {
    let mut builder = SyntaxSet::load_defaults_newlines().into_builder();
    let extras: &[(&str, &str)] = &[(
        "Kotlin.sublime-syntax",
        include_str!("../assets/syntaxes/Kotlin.sublime-syntax"),
    )];
    for (name, src) in extras {
        match SyntaxDefinition::load_from_str(src, true, None) {
            Ok(def) => builder.add(def),
            Err(e) => tracing::warn!(file = name, error = %e, "failed to load extra syntax"),
        }
    }
    builder.build()
});

static THEME: LazyLock<syntect::highlighting::Theme> = LazyLock::new(|| {
    let ts = ThemeSet::load_defaults();
    ts.themes["base16-eighties.dark"].clone()
});

/// Map language tokens that syntect doesn't bundle to a close fallback.
fn resolve_lang(lang: &str) -> &str {
    match lang {
        "tsx" => "typescript",
        "dockerfile" | "Dockerfile" => "bash",
        "toml" => "ini",
        "zsh" => "bash",
        _ => lang,
    }
}

pub fn highlight_code(code: &str, lang: &str, indent: &str) -> Option<Vec<Line<'static>>> {
    let resolved = resolve_lang(lang);
    let syntax = SYNTAX_SET
        .find_syntax_by_token(resolved)
        .or_else(|| SYNTAX_SET.find_syntax_by_token(lang))?;
    let mut h = HighlightLines::new(syntax, &THEME);
    let mut lines = Vec::new();
    for line in LinesWithEndings::from(code) {
        let ranges = h.highlight_line(line, &SYNTAX_SET).ok()?;
        let mut spans: Vec<Span<'static>> = Vec::new();
        if !indent.is_empty() {
            spans.push(Span::raw(indent.to_owned()));
        }
        for (style, text) in ranges {
            let fg = Color::Rgb(style.foreground.r, style.foreground.g, style.foreground.b);
            spans.push(Span::styled(
                text.trim_end_matches('\n').to_owned(),
                Style::default().fg(fg),
            ));
        }
        lines.push(Line::from(spans));
    }
    Some(lines)
}
