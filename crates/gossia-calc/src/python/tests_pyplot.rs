//! Tests of Gossia pyplot (DESIGN.md, "Gossia pyplot, 0.6.0"), steps 16a and 16b:
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
    plt.plot([1, 2], fillstyle='left')

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
        message.contains("does not support the argument fillstyle of plot"),
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

// ------------------------------------------------------------- step 16b

/// The text a formula on `S.text` gives, for a function that returns text.
fn text_of(source: &str, formula: &str) -> String {
    svg_of(source, formula)
}

#[test]
fn bars_stand_on_zero_and_take_tick_labels_colours_and_error_bars() {
    let source = "\
def bars():
    plt.clear()
    plt.bar(['a', 'b', 'c'], [3, 5, 4], color=['red', 'green', 'blue'], yerr=1, capsize=4)
    return plt.svg()

def heights():
    plt.clear()
    bars = plt.bar([1, 2], [3, 5], tick_label=['one', 'two'])
    r = str([b.get_height() for b in bars]) + str(plt.xticks()[1])
    plt.clear()
    return r

def stacked_h():
    plt.clear()
    plt.barh(['x', 'y'], [2, 3], left=[1, 1])
    return plt.svg()
";
    let svg = svg_of(source, "Ch.bars()");
    for colour in ["red", "green", "blue"] {
        assert!(svg.contains(&format!("fill=\"{colour}\"")), "{colour}");
    }
    // The bars stand on 0: the axis does not go below it.
    let t = texts(&svg);
    assert!(t.iter().any(|x| x == "0"), "{t:?}");
    assert!(!t.iter().any(|x| x.starts_with('\u{2212}')), "{t:?}");
    assert_eq!(
        svg.matches("<rect x=").count(),
        3 + 2,
        "three bars, the clip and the frame"
    );
    assert_eq!(text_of(source, "Ch.heights()"), "[3.0, 5.0]['one', 'two']");
    let svg = svg_of(source, "Ch.stacked_h()");
    assert!(texts(&svg).iter().any(|x| x == "x"));
}

#[test]
fn hist_counts_into_bins_and_gives_counts_edges_and_bars() {
    let source = "\
def counts():
    plt.clear()
    n, edges, bars = plt.hist([1, 2, 2, 3, 3, 3, 4], bins=3)
    r = str((n, [round(e, 3) for e in edges], len(bars)))
    plt.clear()
    return r

def density():
    plt.clear()
    n, edges, _ = plt.hist([0, 1, 1, 2], bins=[0, 1, 2], density=True)
    r = str(n)
    plt.clear()
    return r

def two():
    plt.clear()
    plt.hist([[1, 2, 3], [2, 3, 4]], bins=3, label=['a', 'b'], histtype='step')
    plt.legend()
    return plt.svg()
";
    assert_eq!(
        text_of(source, "Ch.counts()"),
        "([1.0, 2.0, 4.0], [1.0, 2.0, 3.0, 4.0], 3)"
    );
    assert_eq!(text_of(source, "Ch.density()"), "[0.25, 0.75]");
    let t = texts(&svg_of(source, "Ch.two()"));
    assert!(
        t.iter().any(|x| x == "a") && t.iter().any(|x| x == "b"),
        "{t:?}"
    );
}

#[test]
fn scatter_has_a_size_and_colour_per_point_and_refuses_colour_maps() {
    let source = "\
def points():
    plt.clear()
    plt.scatter([1, 2, 3], [3, 1, 2], s=[16, 64, 144], c=['red', 'tab:green', '#123456'])
    return plt.svg()

def mapped():
    plt.clear()
    plt.scatter([1, 2], [1, 2], c=[0.1, 0.9])
";
    let svg = svg_of(source, "Ch.points()");
    for (r, colour) in [("2", "red"), ("4", "#2ca02c"), ("6", "#123456")] {
        assert!(
            svg.contains(&format!("r=\"{r}\" fill=\"{colour}\"")),
            "{r} {colour}"
        );
    }
    let message = error_of(source, "Ch.mapped()");
    assert!(
        message.contains("does not support colour maps"),
        "{message}"
    );
}

#[test]
fn a_pie_is_round_without_a_frame_with_labels_and_percentages() {
    let source = "\
def pie():
    plt.clear()
    plt.pie([1, 3], labels=['one', 'three'], autopct='%1.0f%%', startangle=90)
    return plt.svg()

def donut():
    plt.clear()
    plt.pie([1, 1, 2], wedgeprops={'width': 0.3})
    return plt.svg()

def shadow():
    plt.clear()
    plt.pie([1, 2], shadow=True)
";
    let svg = svg_of(source, "Ch.pie()");
    let t = texts(&svg);
    for word in ["one", "three", "25%", "75%"] {
        assert!(t.iter().any(|x| x == word), "{word}: {t:?}");
    }
    assert_eq!(svg.matches(" A").count(), 2, "one arc per wedge");
    assert!(
        !svg.contains(
            "fill=\"none\" stroke=\"currentColor\" stroke-opacity=\"0.8\" stroke-width=\"0.8\"/>"
        ),
        "no frame"
    );
    // An annulus has an inner arc too.
    assert_eq!(svg_of(source, "Ch.donut()").matches(" A").count(), 6);
    let message = error_of(source, "Ch.shadow()");
    assert!(message.contains("does not support shadows"), "{message}");
}

#[test]
fn a_legend_lists_the_labels_in_matplotlibs_order_and_skips_underscores() {
    let source = "\
def legend():
    plt.clear()
    plt.bar([1, 2], [1, 2], label='bars')
    plt.plot([1, 2], [2, 1], label='line')
    plt.plot([1, 2], [1, 1], label='_hidden')
    plt.scatter([1], [1], label='dot')
    plt.legend(loc='lower right', title='Key')
    return plt.svg()

def given():
    plt.clear()
    a, = plt.plot([1, 2])
    b, = plt.plot([2, 1])
    plt.legend([b, a], ['second', 'first'], ncol=2)
    return plt.svg()

def by_labels():
    plt.clear()
    plt.plot([1, 2])
    plt.plot([2, 1])
    plt.legend(['up', 'down'])
    return plt.svg()

def outside():
    plt.clear()
    plt.plot([1, 2], label='x')
    plt.legend(bbox_to_anchor=(1.05, 1))
";
    let t = texts(&svg_of(source, "Ch.legend()"));
    let at = |w: &str| t.iter().position(|x| x == w).unwrap_or(usize::MAX);
    assert!(at("Key") < at("line"), "{t:?}");
    // Lines and collections first, then the bar container.
    assert!(at("line") < at("dot") && at("dot") < at("bars"), "{t:?}");
    assert_eq!(at("_hidden"), usize::MAX);
    let t = texts(&svg_of(source, "Ch.given()"));
    assert!(
        t.iter().position(|x| x == "second") < t.iter().position(|x| x == "first"),
        "{t:?}"
    );
    let t = texts(&svg_of(source, "Ch.by_labels()"));
    assert!(t.iter().any(|x| x == "up") && t.iter().any(|x| x == "down"));
    let message = error_of(source, "Ch.outside()");
    assert!(
        message.contains("does not support the argument bbox_to_anchor of legend"),
        "{message}"
    );
}

#[test]
fn errorbar_step_and_boxplot_draw_their_parts() {
    let source = "\
def errors():
    plt.clear()
    plt.errorbar([1, 2, 3], [1, 2, 3], yerr=[[0.1, 0.2, 0.3], [0.5, 0.5, 0.5]], fmt='o', capsize=3)
    return plt.svg()

def steps():
    plt.clear()
    plt.step([1, 2, 3], [1, 3, 2], where='post')
    return plt.svg()

def boxes():
    plt.clear()
    plt.boxplot([[1, 2, 3, 4, 5, 6, 7, 8, 9, 50], [2, 3, 4]], tick_labels=['A', 'B'])
    return plt.svg()
";
    let svg = svg_of(source, "Ch.errors()");
    assert_eq!(svg.matches("<circle").count(), 3);
    let steps = svg_of(source, "Ch.steps()");
    // Three points as steps after: five corners.
    let path = steps.split("<path d=\"M").nth(1).unwrap();
    let path = &path[..path.find('"').unwrap()];
    assert_eq!(path.matches(" L").count(), 4, "{path}");
    let boxes = svg_of(source, "Ch.boxes()");
    let t = texts(&boxes);
    assert!(
        t.iter().any(|x| x == "A") && t.iter().any(|x| x == "B"),
        "{t:?}"
    );
    // 50 lies beyond the whisker: one flier, an open circle.
    assert_eq!(boxes.matches("<circle").count(), 1);
    assert!(boxes.contains("fill=\"none\" stroke=\"currentColor\""));
    assert!(boxes.contains("stroke=\"#ff7f0e\""), "the median line");
}

#[test]
fn twinx_puts_a_second_y_axis_on_the_right_sharing_x() {
    let source = "\
def twin():
    plt.clear()
    fig, ax = plt.subplots()
    ax.plot([1, 2, 3], [1, 2, 3])
    ax.set_ylabel('left')
    ax2 = ax.twinx()
    ax2.plot([1, 5], [100, 300], 'r')
    ax2.set_ylabel('right')
    return plt.svg()
";
    let svg = svg_of(source, "Ch.twin()");
    let t = texts(&svg);
    assert!(t.iter().any(|x| x == "left") && t.iter().any(|x| x == "right"));
    // The x axis reaches 5 for the twin's data; its labels are on the right.
    assert!(t.iter().any(|x| x == "5"), "{t:?}");
    assert!(t.iter().any(|x| x == "300"), "{t:?}");
    assert!(svg.contains("text-anchor=\"start\" fill=\"currentColor\" fill-opacity=\"0.8\">300<"));
    assert_eq!(svg.matches("<clipPath").count(), 1, "one plot box");
}

#[test]
fn shared_axes_have_one_range_and_tick_labels_only_outside() {
    let source = "\
def shared():
    plt.clear()
    fig, axs = plt.subplots(2, 1, sharex=True)
    axs[0].plot([0, 10], [0, 1])
    axs[1].plot([0, 2], [0, 1])
    return plt.svg()
";
    let t = texts(&svg_of(source, "Ch.shared()"));
    // Both plots run to 10, and only the lower one shows its x labels.
    assert_eq!(t.iter().filter(|x| *x == "10").count(), 1, "{t:?}");
    assert_eq!(t.iter().filter(|x| *x == "0").count(), 1, "{t:?}");
}

#[test]
fn text_annotate_and_lines_across_are_drawn() {
    let source = "\
def notes():
    plt.clear()
    plt.plot([0, 10], [0, 10])
    plt.axhline(5, color='k', linestyle='--')
    plt.axvspan(2, 3, alpha=0.2)
    plt.text(1, 9, 'note', fontsize=12, ha='left')
    plt.text(0.5, 0.5, 'middle', transform=plt.gca().transAxes, ha='center')
    plt.annotate('peak', xy=(8, 8), xytext=(4, 9), arrowprops={'arrowstyle': '->'})
    plt.annotate('old', xy=(2, 2), xytext=(5, 1), arrowprops={'facecolor': 'black', 'shrink': 0.05})
    return plt.svg()
";
    let svg = svg_of(source, "Ch.notes()");
    let t = texts(&svg);
    for word in ["note", "middle", "peak", "old"] {
        assert!(t.iter().any(|x| x == word), "{word}: {t:?}");
    }
    assert!(svg.contains("font-size=\"12\" text-anchor=\"start\""));
    assert!(svg.contains("stroke=\"#000000\" stroke-width=\"1.5\" stroke-dasharray"));
    assert!(svg.contains("opacity=\"0.2\""));
    // The old style arrow has a filled head.
    assert!(svg.contains("<polygon points="), "arrowhead");
}

#[test]
fn handles_can_be_restyled_and_unknown_methods_are_refused() {
    let source = "\
def restyled():
    plt.clear()
    line, = plt.plot([1, 2], [1, 2])
    line.set_color('purple')
    line.set_linewidth(3)
    line.set_label('mine')
    bars = plt.bar([1, 2], [1, 2])
    bars[0].set_color('orange')
    plt.legend()
    return plt.svg()

def refused():
    plt.clear()
    line, = plt.plot([1, 2])
    line.set_dash_capstyle('round')
";
    let svg = svg_of(source, "Ch.restyled()");
    assert!(svg.contains("stroke=\"purple\" stroke-width=\"3\""));
    assert!(svg.contains("fill=\"orange\""));
    assert!(texts(&svg).iter().any(|x| x == "mine"));
    let message = error_of(source, "Ch.refused()");
    assert!(
        message.contains("does not support Line.set_dash_capstyle"),
        "{message}"
    );
}

#[test]
fn equal_aspect_and_axis_off() {
    let source = "\
def equal():
    plt.clear()
    plt.figure(width=400, height=200)
    plt.plot([0, 1], [0, 1])
    plt.axis('equal')
    return plt.svg()

def off():
    plt.clear()
    plt.plot([0, 1], [0, 1])
    plt.axis('off')
    return plt.svg()
";
    let svg = svg_of(source, "Ch.equal()");
    let at = svg.find("<clipPath").unwrap();
    let rect = &svg[at..at + svg[at..].find("/>").unwrap()];
    let attr = |name: &str| -> f64 {
        let i = rect.find(&format!("{name}=\"")).unwrap() + name.len() + 2;
        rect[i..i + rect[i..].find('"').unwrap()].parse().unwrap()
    };
    assert!(
        (attr("width") - attr("height")).abs() < 0.5,
        "a square box: {rect}"
    );
    let off = svg_of(source, "Ch.off()");
    assert!(texts(&off).is_empty(), "no tick labels");
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

def bars():
    plt.clear()
    labels = ['North', 'East', 'South', 'West']
    xs = range(len(labels))
    fig, ax = plt.subplots()
    ax.bar([x - 0.2 for x in xs], [20, 34, 30, 35], 0.4, label='2025')
    ax.bar([x + 0.2 for x in xs], [25, 32, 34, 20], 0.4, label='2026', yerr=[2, 3, 4, 1], capsize=3)
    ax.set_xticks(list(xs), labels)
    ax.set_ylabel('Sales')
    ax.set_title('Sales by region')
    ax.legend()
    return plt.svg()

def stacked():
    plt.clear()
    months = ['Jan', 'Feb', 'Mar', 'Apr']
    a = [3, 4, 5, 6]
    b = [2, 2, 3, 1]
    plt.bar(months, a, label='Online', color='tab:green')
    plt.bar(months, b, bottom=a, label='Shop', color='tab:olive')
    plt.legend(loc='upper left')
    plt.title('Stacked')
    return plt.svg()

def barh():
    plt.clear()
    people = ['Tom', 'Dick', 'Harry', 'Slim', 'Jim']
    plt.barh(people, [8, 11, 4, 9, 12], xerr=[1, 2, 1, 0.5, 1], align='center', color='#4c72b0')
    plt.xlabel('Performance')
    plt.title('How fast do you want to go today?')
    return plt.svg()

def scatter():
    plt.clear()
    xs = [((i * 37) % 100) / 10.0 for i in range(50)]
    ys = [((i * 53) % 100) / 10.0 for i in range(50)]
    sizes = [20 + (i * 7) % 200 for i in range(50)]
    colours = ['tab:blue' if i % 3 else 'tab:red' for i in range(50)]
    plt.scatter(xs, ys, s=sizes, c=colours, alpha=0.5, label='points')
    plt.scatter([5], [5], marker='*', s=200, color='k', label='centre')
    plt.legend()
    plt.title('Scatter')
    return plt.svg()

def hist():
    plt.clear()
    a = [math.sin(i * 1.3) * 3 + math.cos(i * 0.7) * 2 for i in range(400)]
    b = [math.sin(i * 0.9) * 2 + 2 for i in range(400)]
    plt.hist([a, b], bins=15, label=['first', 'second'])
    plt.hist(a, bins=15, histtype='step', color='k', linewidth=1)
    plt.legend()
    plt.title('Histograms')
    return plt.svg()

def pie():
    plt.clear()
    fig, axs = plt.subplots(1, 2, width=500, height=260)
    sizes = [15, 30, 45, 10]
    labels = ['Frogs', 'Hogs', 'Dogs', 'Logs']
    axs[0].pie(sizes, labels=labels, autopct='%1.1f%%', explode=(0, 0.1, 0, 0), startangle=90)
    axs[0].set_title('Pie')
    axs[1].pie(sizes, labels=labels, wedgeprops=dict(width=0.4, edgecolor='w'))
    axs[1].set_title('Donut')
    return plt.svg()

def fill():
    plt.clear()
    xs = [i / 10.0 for i in range(0, 63)]
    s = [math.sin(x) for x in xs]
    c = [math.cos(x) for x in xs]
    plt.plot(xs, s, label='sin')
    plt.plot(xs, c, label='cos')
    plt.fill_between(xs, s, c, where=[a > b for a, b in zip(s, c)], alpha=0.3, color='tab:green',
                     label='sin above')
    plt.axhline(0, color='gray', linewidth=0.8)
    plt.axvspan(2, 3, alpha=0.15, color='tab:red')
    plt.legend(loc='lower left')
    plt.title('fill_between')
    return plt.svg()

def errors():
    plt.clear()
    xs = [1, 2, 3, 4, 5, 6]
    ys = [2.1, 3.9, 6.2, 7.8, 10.1, 12.2]
    plt.errorbar(xs, ys, yerr=[0.5, 0.4, 0.8, 0.3, 0.9, 0.6], fmt='o', capsize=4, label='measured')
    plt.step(xs, [2, 4, 6, 8, 10, 12], where='mid', label='steps')
    plt.legend()
    plt.title('Error bars and steps')
    return plt.svg()

def boxes():
    plt.clear()
    data = [[math.sin(i * k) * k + k * 2 for i in range(60)] + [k * 6] for k in (1, 2, 3)]
    bp = plt.boxplot(data, tick_labels=['one', 'two', 'three'], patch_artist=True, showmeans=True)
    for box, colour in zip(bp['boxes'], ['pink', 'lightblue', 'lightgreen']):
        box.set_facecolor(colour)
    plt.title('Box plots')
    return plt.svg()

def twin():
    plt.clear()
    fig, ax1 = plt.subplots()
    months = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
    ax1.bar(months, [5, 7, 9, 14, 18, 21, 23, 22, 19, 14, 9, 6], color='tab:orange', label='Temperature')
    ax1.set_ylabel('Temperature (C)')
    ax2 = ax1.twinx()
    ax2.plot(months, [80, 60, 55, 45, 50, 60, 70, 80, 75, 95, 100, 90], 'b-o', label='Rain')
    ax2.set_ylabel('Rain (mm)')
    ax1.set_title('Climate')
    h1, l1 = ax1.get_legend_handles_labels()
    h2, l2 = ax2.get_legend_handles_labels()
    ax2.legend(h1 + h2, l1 + l2, loc='upper left')
    return plt.svg()

def notes():
    plt.clear()
    xs = [i / 10.0 for i in range(0, 50)]
    ys = [math.cos(2 * math.pi * x) * math.exp(-x / 3) for x in xs]
    plt.plot(xs, ys)
    plt.annotate('local max', xy=(2, 0.5), xytext=(3, 0.8), arrowprops=dict(facecolor='black', shrink=0.05))
    plt.annotate('a dip', xy=(0.5, -0.85), xytext=(1.2, -0.9), arrowprops=dict(arrowstyle='->'))
    plt.text(0.95, 0.95, 'damped', transform=plt.gca().transAxes, ha='right', va='top', fontsize=12)
    plt.ylim(-1.2, 1.2)
    plt.title('Annotations')
    return plt.svg()

def shared():
    plt.clear()
    fig, axs = plt.subplots(2, 2, sharex=True, sharey=True)
    for i, ax in enumerate(axs.flat):
        ax.plot([1, 2, 3, 4], [v * (i + 1) for v in (1, 3, 2, 4)])
        ax.set_title('Plot %d' % (i + 1))
    fig.suptitle('Shared axes')
    return plt.svg()
";
    for name in [
        "lines", "dates", "log", "grid", "bars", "stacked", "barh", "scatter", "hist", "pie",
        "fill", "errors", "boxes", "twin", "notes", "shared",
    ] {
        let svg = svg_of(
            source,
            &format!("_chart_svg(_chart_size(400, 300) or Ch.{name}())"),
        );
        std::fs::write(std::path::Path::new(&dir).join(format!("{name}.svg")), svg).unwrap();
    }
}
