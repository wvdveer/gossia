//! Tests of Gossia pyplot (DESIGN.md, "Gossia pyplot, 0.6.0"), step 16a:
//! figures and axes, `plt.clear()` and `plt.svg()`, ticks, text measuring
//! and dates, run through the real calculation layer and engine.

use gossia_core::{ErrorKind, Value};

use super::tests::Book;

/// A book with the module `Ch`, whose source is `source`.
fn book(source: &str) -> Book {
    let mut b = Book::new();
    b.calc.add_module("Ch", source).unwrap();
    b
}

/// The text a formula on `S.text` gives, run after the module `Ch`.
fn svg_of(source: &str, formula: &str) -> String {
    let mut b = book(source);
    b.formula("text", formula);
    b.run();
    match b.get("text") {
        Some(Value::Str(s)) => s.to_string(),
        other => panic!("not text: {other:?} ({})", message_or_none(&b)),
    }
}

fn message_or_none(b: &Book) -> String {
    match b.get("text") {
        Some(Value::Error(e)) => e.message,
        _ => String::new(),
    }
}

/// The error message a formula on `S.text` ends in.
fn error_of(source: &str, formula: &str) -> String {
    let mut b = book(source);
    b.formula("text", formula);
    b.run();
    b.message("text")
}

/// The texts of the `<text>` elements of an SVG, in order.
fn texts(svg: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = svg;
    while let Some(start) = rest.find("<text") {
        let after = &rest[start..];
        let open = after.find('>').unwrap() + 1;
        let close = after.find("</text>").unwrap();
        out.push(
            after[open..close]
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&amp;", "&"),
        );
        rest = &after[close..];
    }
    out
}

const LINE: &str = "\
def line(width=400, height=300):
    plt.clear()
    plt.plot([1, 2, 3, 4], [10, 30, 20, 40], label='Sales')
    plt.title('Sales')
    plt.xlabel('Quarter')
    plt.ylabel('Euro')
    return plt.svg()
";

#[test]
fn a_chart_function_draws_a_figure_and_svg_gives_its_text() {
    let svg = svg_of(LINE, "Ch.line()");
    assert!(
        svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"400\" height=\"300\"")
    );
    assert!(svg.ends_with("</svg>"));
    // The theme: text in the chart's colour, a background of class
    // `background`.
    assert!(svg.contains("class=\"background\""));
    assert!(svg.contains("fill=\"currentColor\""));
    // The first series is matplotlib's first colour.
    assert!(svg.contains("stroke=\"#1f77b4\""));
    let t = texts(&svg);
    for label in ["Sales", "Quarter", "Euro"] {
        assert!(t.iter().any(|x| x == label), "{label} in {t:?}");
    }
    // Round ticks: 1 to 4 along x, tens up the y axis.
    for tick in ["1", "2", "3", "4", "10", "20", "30", "40"] {
        assert!(t.iter().any(|x| x == tick), "{tick} in {t:?}");
    }
}

#[test]
fn the_figure_is_the_size_of_the_chart_unless_given_one() {
    // Gossia tells pyplot the chart's size before calling the function.
    let source = "def f():\n    plt.clear()\n    plt.plot([1, 2])\n    return plt.svg()\n";
    let svg = svg_of(source, "_chart_svg(_chart_size(250, 120) or Ch.f())");
    assert!(svg.contains("width=\"250\" height=\"120\""), "{svg:.120}");
    let source = "def f():\n    plt.clear()\n    plt.figure(width=500, figsize=(4, 1))\n    plt.plot([1, 2])\n    return plt.svg()\n";
    let svg = svg_of(source, "Ch.f()");
    assert!(svg.contains("width=\"500\" height=\"125\""), "{svg:.120}");
}

#[test]
fn clear_forgets_a_figure_left_over_and_svg_closes_the_one_it_draws() {
    let source = "\
def broken():
    plt.plot([1, 2, 3])
    raise ValueError('stopped')

def after():
    plt.clear()
    plt.plot([5, 6])
    return plt.svg()

def without_clear():
    plt.plot([5, 6])
    return plt.svg()

def nothing():
    plt.clear()
    return plt.svg()

def twice():
    plt.clear()
    plt.plot([1, 2])
    plt.svg()
    return plt.svg()
";
    // A chart that fails leaves its figure behind (`sum` runs first, since
    // `text` reads it).
    let after_broken = |f: &str| {
        let mut b = book(source);
        b.formula("sum", "Ch.broken()");
        b.formula("text", &format!("Ch.{f}() if ISERROR(S.sum) else ''"));
        b.run();
        match b.get("text") {
            Some(Value::Str(s)) => s,
            other => panic!("{other:?}"),
        }
    };
    // clear() starts afresh: one line.
    let svg = after_broken("after");
    assert_eq!(svg.matches("stroke=\"#1f77b4\"").count(), 1, "{svg}");
    assert_eq!(svg.matches("#ff7f0e").count(), 0);
    // Without clear() the left-over figure is drawn into: a second line.
    let svg = after_broken("without_clear");
    assert!(svg.contains("#ff7f0e"), "{svg}");
    assert!(error_of(source, "Ch.nothing()").contains("there is no figure to draw"));
    assert!(error_of(source, "Ch.twice()").contains("there is no figure to draw"));
}

#[test]
fn a_chart_function_must_finish_with_svg_and_there_is_no_show() {
    let source = "\
def returns_figure():
    plt.clear()
    fig = plt.figure()
    plt.plot([1, 2])
    return fig

def returns_nothing():
    plt.clear()
    plt.plot([1, 2])

def shows():
    plt.clear()
    plt.plot([1, 2])
    plt.show()

def saves():
    plt.clear()
    plt.plot([1, 2])
    plt.savefig('chart.png')
";
    for f in ["returns_figure", "returns_nothing"] {
        let message = error_of(source, &format!("_chart_svg(Ch.{f}())"));
        assert!(
            message.contains("must finish with return plt.svg()"),
            "{f}: {message}"
        );
    }
    let message = error_of(source, "Ch.shows()");
    assert!(message.contains("has no show()"), "{message}");
    assert!(message.contains("return plt.svg()"), "{message}");
    let message = error_of(source, "Ch.saves()");
    assert!(message.contains("cannot write files"), "{message}");
}

#[test]
fn what_is_not_covered_is_refused_by_name_and_harmless_arguments_are_ignored() {
    let source = "\
def image():
    plt.clear()
    plt.imshow([[1]])

def axes_method():
    plt.clear()
    ax = plt.gca()
    ax.contour([[1]])

def argument():
    plt.clear()
    plt.plot([1, 2], drawstyle='steps')

def ignored():
    plt.clear()
    plt.figure(dpi=200)
    plt.plot([1, 2], zorder=3, antialiased=False, rasterized=True, picker=5, clip_on=False, snap=True)
    return plt.svg()
";
    let message = error_of(source, "Ch.image()");
    assert!(
        message.contains("gossia.pyplot does not support imshow"),
        "{message}"
    );
    let message = error_of(source, "Ch.axes_method()");
    assert!(
        message.contains("does not support Axes.contour"),
        "{message}"
    );
    let message = error_of(source, "Ch.argument()");
    assert!(
        message.contains("does not support the argument drawstyle of plot"),
        "{message}"
    );
    assert!(svg_of(source, "Ch.ignored()").contains("<path"));
}

#[test]
fn plt_is_there_without_import_and_gossia_pyplot_can_be_imported() {
    let source = "\
import gossia.pyplot as gp
from gossia import pyplot
from gossia.pyplot import plot, svg, clear

def same():
    return gp is plt and pyplot is plt

def drawn():
    clear()
    plot([1, 2])
    return svg()
";
    let mut b = book(source);
    b.formula("text", "str(Ch.same())");
    b.run();
    assert_eq!(b.get("text"), Some(Value::Str("True".into())));
    assert!(svg_of(source, "Ch.drawn()").starts_with("<svg"));
}

#[test]
fn text_is_measured_so_a_long_tick_label_moves_the_plot_box() {
    let source = "\
def narrow():
    plt.clear()
    plt.plot([1, 2], [1, 2])
    return plt.svg()

def wide():
    plt.clear()
    plt.plot([1, 2], [100000, 200000])
    return plt.svg()
";
    // The plot box is the frame's rect, after the clip path's.
    let left = |svg: &str| -> f64 {
        let frame = svg.rfind("<rect x=\"").unwrap();
        let rest = &svg[frame + 9..];
        rest[..rest.find('"').unwrap()].parse().unwrap()
    };
    assert!(left(&svg_of(source, "Ch.wide()")) > left(&svg_of(source, "Ch.narrow()")) + 15.0);
}

#[test]
fn ticks_are_round_with_the_decimals_they_need_and_an_offset_for_large_numbers() {
    let source = "\
def small():
    plt.clear()
    plt.plot([0, 1], [0, 1])
    return plt.svg()

def large():
    plt.clear()
    plt.plot([0, 1], [0, 3000000])
    return plt.svg()

def negative():
    plt.clear()
    plt.plot([0, 1], [-2, 2])
    return plt.svg()

def log():
    plt.clear()
    plt.plot([1, 2, 3], [1, 100, 10000])
    plt.yscale('log')
    return plt.svg()
";
    let t = texts(&svg_of(source, "Ch.small()"));
    for tick in ["0.0", "0.2", "0.4", "1.0"] {
        assert!(t.iter().any(|x| x == tick), "{tick} in {t:?}");
    }
    let t = texts(&svg_of(source, "Ch.large()"));
    assert!(t.iter().any(|x| x == "\u{d7}10\u{2076}"), "{t:?}");
    assert!(t.iter().any(|x| x == "3.0"), "{t:?}");
    let t = texts(&svg_of(source, "Ch.negative()"));
    assert!(
        t.iter().any(|x| x == "\u{2212}2"),
        "a minus sign, not a hyphen: {t:?}"
    );
    let t = texts(&svg_of(source, "Ch.log()"));
    for tick in ["10\u{2070}", "10\u{b2}", "10\u{2074}"] {
        assert!(t.iter().any(|x| x == tick), "{tick} in {t:?}");
    }
}

#[test]
fn limits_ticks_and_rotation_set_by_the_user_are_kept() {
    let source = "\
def limits():
    plt.clear()
    plt.plot([1, 2, 3], [1, 2, 3])
    plt.xlim(0, 10)
    return str(plt.xlim())

def ticks():
    plt.clear()
    plt.plot([1, 2, 3], [1, 2, 3])
    plt.xticks([1, 2, 3], ['one', 'two', 'three'], rotation=45)
    return plt.svg()
";
    let mut b = book(source);
    b.formula("text", "Ch.limits()");
    b.run();
    assert_eq!(b.get("text"), Some(Value::Str("(0.0, 10.0)".into())));
    let svg = svg_of(source, "Ch.ticks()");
    let t = texts(&svg);
    for label in ["one", "two", "three"] {
        assert!(t.iter().any(|x| x == label), "{label} in {t:?}");
    }
    assert!(svg.contains("rotate(-45"), "rotated labels");
}

#[test]
fn text_on_an_axis_is_categories_in_the_order_first_seen() {
    let source = "\
def months():
    plt.clear()
    plt.plot(['Jan', 'Feb', 'Mar'], [3, 1, 2], 'o-')
    return plt.svg()

def mixed():
    plt.clear()
    plt.plot(['Jan', 'Feb'], [1, 2])
    plt.plot([1, 2], [1, 2])
";
    let svg = svg_of(source, "Ch.months()");
    let t = texts(&svg);
    let at = |name: &str| t.iter().position(|x| x == name).unwrap();
    assert!(at("Jan") < at("Feb") && at("Feb") < at("Mar"), "{t:?}");
    assert_eq!(svg.matches("<circle").count(), 3, "a marker on each point");
    let message = error_of(source, "Ch.mixed()");
    assert!(
        message.contains("the x axis has text, so it cannot take numbers"),
        "{message}"
    );
}

#[test]
fn dates_on_an_axis_get_ticks_on_whole_months_years_or_days() {
    let source = "\
def months():
    plt.clear()
    days = [datetime.date(2026, 1, 1) + datetime.timedelta(days=d) for d in range(0, 200, 10)]
    plt.plot(days, list(range(len(days))))
    return plt.svg()

def years():
    plt.clear()
    plt.plot([datetime.date(2000, 6, 1), datetime.date(2030, 6, 1)], [1, 2])
    return plt.svg()

def days():
    plt.clear()
    plt.plot([datetime.date(2026, 3, 1), datetime.date(2026, 3, 8)], [1, 2])
    return plt.svg()

def hours():
    plt.clear()
    plt.plot([datetime.datetime(2026, 3, 1, 8), datetime.datetime(2026, 3, 1, 18)], [1, 2])
    return plt.svg()

def limit():
    plt.clear()
    plt.plot([datetime.date(2026, 3, 1), datetime.date(2026, 3, 8)], [1, 2])
    return str(plt.xlim(datetime.date(2026, 2, 1), datetime.date(2026, 4, 1)))
";
    let t = texts(&svg_of(source, "Ch.months()"));
    // The first tick has the year; the others only the month.
    assert_eq!(t[0], "Jan 2026", "{t:?}");
    assert!(t.iter().any(|x| x == "Mar"), "{t:?}");
    let t = texts(&svg_of(source, "Ch.years()"));
    assert!(
        t.iter().any(|x| x == "2010") && t.iter().any(|x| x == "2020"),
        "{t:?}"
    );
    let t = texts(&svg_of(source, "Ch.days()"));
    assert!(
        t.iter().any(|x| x == "2 Mar") || t.iter().any(|x| x == "3 Mar"),
        "{t:?}"
    );
    let t = texts(&svg_of(source, "Ch.hours()"));
    assert!(t.iter().any(|x| x == "12:00"), "{t:?}");
    let mut b = book(source);
    b.formula("text", "Ch.limit()");
    b.run();
    assert_eq!(
        b.get("text"),
        Some(Value::Str(
            "(datetime.datetime(2026, 2, 1, 0, 0), datetime.datetime(2026, 4, 1, 0, 0))".into()
        ))
    );
}

#[test]
fn subplots_give_a_grid_of_axes_indexed_by_row_and_column() {
    let source = "\
def grid():
    plt.clear()
    fig, axs = plt.subplots(2, 2, width=400, height=400)
    axs[0, 0].plot([1, 2], [1, 2])
    axs[0][1].plot([1, 2], [2, 1])
    axs[1, 0].set_title('Bottom left')
    for ax in axs.flat:
        ax.grid(True)
    fig.suptitle('Four')
    return plt.svg()

def shape():
    plt.clear()
    fig, axs = plt.subplots(1, 3)
    return str((len(axs), axs.shape))
";
    let svg = svg_of(source, "Ch.grid()");
    assert_eq!(svg.matches("<clipPath").count(), 4, "four plot boxes");
    let t = texts(&svg);
    assert!(t.iter().any(|x| x == "Four") && t.iter().any(|x| x == "Bottom left"));
    let mut b = book(source);
    b.formula("text", "Ch.shape()");
    b.run();
    assert_eq!(b.get("text"), Some(Value::Str("(3, (3,))".into())));
}

#[test]
fn the_svg_parses_and_two_figures_never_share_a_clip_path_id() {
    let source = "\
def a():
    plt.clear()
    plt.plot([1, 2, 3], [3, 1, 2], 'r--')
    return plt.svg()

def b():
    plt.clear()
    plt.plot([1, 2, 3], [1, 2, 3], 'gs:')
    return plt.svg()
";
    let a = svg_of(source, "Ch.a()");
    let b = svg_of(source, "Ch.b()");
    let id = |svg: &str| {
        let at = svg.find("clipPath id=\"").unwrap() + 13;
        svg[at..at + svg[at..].find('"').unwrap()].to_owned()
    };
    assert_ne!(id(&a), id(&b));
    assert!(a.contains("stroke=\"#ff0000\"") && a.contains("stroke-dasharray"));
    assert!(b.contains("#008000") && b.contains("<rect x="));
    for svg in [&a, &b] {
        let opened = svg.matches("<g").count();
        assert_eq!(opened, svg.matches("</g>").count());
    }
}

#[test]
fn values_from_a_tab_are_drawn_as_their_plain_values() {
    let source = "\
def from_tab(values):
    plt.clear()
    plt.plot(values)
    return plt.svg()
";
    let t = texts(&svg_of(source, "Ch.from_tab(S.nums)"));
    assert!(t.iter().any(|x| x == "30"), "{t:?}");
    let message = error_of(source, "Ch.from_tab(S.tags + [True])");
    assert!(!message.is_empty());
}

#[test]
fn a_figure_too_small_for_its_labels_says_so() {
    let source = "\
def tiny():
    plt.clear()
    plt.figure(width=30, height=20)
    plt.plot([1, 2])
    plt.title('A title')
    return plt.svg()
";
    let mut b = book(source);
    b.formula("text", "Ch.tiny()");
    b.run();
    assert_eq!(b.error("text"), Some(ErrorKind::Value));
    assert!(b.message("text").contains("too small"));
}

/// Writes example charts as SVG files into the folder named by
/// `GOSSIA_PYPLOT_GALLERY`, to look at by eye:
/// `GOSSIA_PYPLOT_GALLERY=dir cargo test -p gossia-calc -- --ignored write_gallery`.
#[test]
#[ignore = "writes files for looking at by eye"]
fn write_gallery() {
    let Ok(dir) = std::env::var("GOSSIA_PYPLOT_GALLERY") else {
        return;
    };
    let source = "\
def lines():
    plt.clear()
    plt.plot([1, 2, 3, 4, 5], [10, 30, 20, 45, 38], 'o-', label='Sales')
    plt.plot([1, 2, 3, 4, 5], [15, 25, 30, 35, 40], 'r--', label='Target')
    plt.title('Sales and target')
    plt.xlabel('Month')
    plt.ylabel('Euro')
    plt.grid(True)
    return plt.svg()

def dates():
    plt.clear()
    days = [datetime.date(2026, 1, 1) + datetime.timedelta(days=d) for d in range(0, 365, 7)]
    plt.plot(days, [math.sin(i / 8.0) * 100 + 2000 for i in range(len(days))])
    plt.title('Weekly, 2026')
    return plt.svg()

def log():
    plt.clear()
    xs = list(range(1, 30))
    plt.plot(xs, [2 ** x for x in xs], 's:', markersize=3)
    plt.yscale('log')
    plt.title('Doubling')
    return plt.svg()

def grid():
    plt.clear()
    fig, axs = plt.subplots(2, 2)
    axs[0, 0].plot([1, 2, 3], [1, 4, 9])
    axs[0, 1].plot(['Mon', 'Tue', 'Wed', 'Thu', 'Fri'], [3, 5, 2, 6, 4], 'D-')
    axs[1, 0].plot([0, 1], [0, 3000000])
    axs[1, 1].plot([1, 2, 3], [3, 1, 2])
    axs[1, 1].set_xticks([1, 2, 3], ['first', 'second', 'third'], rotation=30)
    fig.suptitle('Four plots')
    return plt.svg()
";
    for name in ["lines", "dates", "log", "grid"] {
        let svg = svg_of(
            source,
            &format!("_chart_svg(_chart_size(400, 300) or Ch.{name}())"),
        );
        std::fs::write(std::path::Path::new(&dir).join(format!("{name}.svg")), svg).unwrap();
    }
}
