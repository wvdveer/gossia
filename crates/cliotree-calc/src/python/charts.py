# The built-in chart types (DESIGN.md, Reports, "Chart functions"): bar,
# line, pie and scatter. Each is a function that takes the data to draw and
# returns SVG, as a chart type of your own does; copy one as the start of a
# new chart type in a module of the workbook:
#
#     @chart("Funnel")
#     def funnel(stages, counts, title=None, *, width, height):
#         ...
#         return svg_text
#
# A report passes `width` and `height`, the size of the chart in points.
# The parameters before them are the boxes on the chart's property page; one
# with a default may be left empty.

import math as _math

# The colours of the series, in order (never cycled: a ninth series is an
# error), and of the rest of the chart. They are for the light page of a
# report, which is how it is printed.
_SERIES = ['#2a78d6', '#eb6834', '#1baf7a', '#eda100',
           '#e87ba4', '#008300', '#6250d6', '#e34948']
_SURFACE = '#fcfcfb'
_TEXT = '#0b0b0b'
_MUTED = '#52514e'
_GRID = '#e4e3df'
_FONT = 'Inter'


def _esc(text):
    return (str(text).replace('&', '&amp;').replace('<', '&lt;')
            .replace('>', '&gt;').replace('"', '&quot;'))


def _number(value, what):
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise TypeError('%s must be numbers, not %r' % (what, value))
    if value != value or value in (float('inf'), float('-inf')):
        raise TypeError('%s must be finite numbers' % what)
    return float(value)


def _label(value):
    if value is None:
        return ''
    if hasattr(value, 'isoformat'):
        return value.isoformat()
    if isinstance(value, float) and value == int(value):
        return str(int(value))
    return str(value)


def _list(values, what):
    # A node of a tab, or a list with nodes in it, as plain values.
    values = PLAIN(values)
    if isinstance(values, (str, bytes, dict)):
        raise TypeError('%s must be a list' % what)
    try:
        return list(values)
    except TypeError:
        raise TypeError('%s must be a list' % what) from None


def _series(values, series, what='values'):
    """The values as a list of series, each a list of numbers, with their
    names: `values` is one list of numbers, or a list of lists."""
    values = _list(values, what)
    if values and all(isinstance(v, (list, tuple)) for v in values):
        lists = [[_number(x, what) for x in _list(v, what)] for v in values]
    else:
        lists = [[_number(x, what) for x in values]]
    if series is None:
        names = [None] if len(lists) == 1 else ['Series %d' % (i + 1) for i in range(len(lists))]
    else:
        names = [_label(n) for n in _list(series, 'series')]
        if len(names) != len(lists):
            raise ValueError('there are %d names in series for %d series' % (len(names), len(lists)))
    if len(lists) > len(_SERIES):
        raise ValueError('a chart shows at most %d series' % len(_SERIES))
    return lists, names


def _nice(lo, hi, count=5):
    """Round ticks from lo to hi, about `count` of them."""
    if lo == hi:
        lo, hi = (lo - 1, hi + 1) if lo == 0 else (min(0.0, lo), max(0.0, hi))
    raw = (hi - lo) / max(count, 1)
    power = 10 ** _math.floor(_math.log10(raw))
    step = power
    for m in (1, 2, 2.5, 5, 10):
        if m * power >= raw:
            step = m * power
            break
    start = _math.floor(lo / step) * step
    end = _math.ceil(hi / step) * step
    ticks = []
    t = start
    while t <= end + step / 2:
        ticks.append(round(t, 10))
        t += step
    return ticks


def _tick_text(t):
    if t == int(t):
        n = int(t)
        return '{:,}'.format(n) if abs(n) >= 10000 else str(n)
    return ('%.6g' % t)


def _text_width(text, size):
    # Close enough for Inter to keep labels apart.
    return len(text) * size * 0.55


class _Svg:
    # Made by _new_svg: a module may not define double-underscore methods.
    def add(self, part):
        self.parts.append(part)

    def text(self, x, y, text, size=9, anchor='start', color=_MUTED, weight=None):
        w = ' font-weight="%s"' % weight if weight else ''
        self.add('<text x="%.1f" y="%.1f" font-size="%g" text-anchor="%s" fill="%s"%s>%s</text>'
                 % (x, y, size, anchor, color, w, _esc(text)))

    def done(self):
        return ('<svg xmlns="http://www.w3.org/2000/svg" width="%g" height="%g" '
                'viewBox="0 0 %g %g" font-family="%s">'
                % (self.width, self.height, self.width, self.height, _FONT)
                + '<rect width="100%%" height="100%%" fill="%s"/>' % _SURFACE
                + ''.join(self.parts) + '</svg>')


def _new_svg(width, height):
    svg = _Svg()
    svg.width = float(width)
    svg.height = float(height)
    svg.parts = []
    return svg


def _frame(width, height, title, names):
    """The chart with its title and legend, and the box left for the plot:
    left, top, right, bottom."""
    svg = _new_svg(width, height)
    top = 8.0
    if title:
        svg.text(8, top + 11, title, size=11, color=_TEXT, weight='600')
        top += 18
    shown = [n for n in names if n]
    if len(names) > 1 and shown:
        x = 8.0
        for i, name in enumerate(names):
            svg.add('<rect x="%.1f" y="%.1f" width="8" height="8" rx="2" fill="%s"/>'
                    % (x, top + 2, _SERIES[i]))
            svg.text(x + 11, top + 9, name, size=9, color=_TEXT)
            x += 11 + _text_width(name, 9) + 12
        top += 16
    return svg, (8.0, top + 4, width - 8.0, height - 6.0)


def _value_axis(svg, ticks, left, top, right, bottom, horizontal=False):
    """Grid lines and tick labels; returns the place of a value."""
    lo, hi = ticks[0], ticks[-1]
    if horizontal:
        def place(v):
            return left + (v - lo) / (hi - lo) * (right - left)
        for t in ticks:
            x = place(t)
            svg.add('<line x1="%.1f" y1="%.1f" x2="%.1f" y2="%.1f" stroke="%s" stroke-width="1"/>'
                    % (x, top, x, bottom, _GRID))
            svg.text(x, bottom + 11, _tick_text(t), anchor='middle')
    else:
        def place(v):
            return bottom - (v - lo) / (hi - lo) * (bottom - top)
        for t in ticks:
            y = place(t)
            svg.add('<line x1="%.1f" y1="%.1f" x2="%.1f" y2="%.1f" stroke="%s" stroke-width="1"/>'
                    % (left, y, right, y, _GRID))
            svg.text(left - 4, y + 3, _tick_text(t), anchor='end')
    return place


def _bar_path(x, y0, y1, w, horizontal):
    """A bar from y0 (the baseline) to y1, square at the baseline and with
    rounded corners at its end."""
    r = min(4.0, w / 2, abs(y1 - y0))
    if horizontal:
        # x is the top of the bar, y the value axis.
        x0, x1, top, h = y0, y1, x, w
        if x1 >= x0:
            return ('M%.1f %.1f H%.1f Q%.1f %.1f %.1f %.1f V%.1f Q%.1f %.1f %.1f %.1f H%.1f Z'
                    % (x0, top, x1 - r, x1, top, x1, top + r, top + h - r,
                       x1, top + h, x1 - r, top + h, x0))
        return ('M%.1f %.1f H%.1f Q%.1f %.1f %.1f %.1f V%.1f Q%.1f %.1f %.1f %.1f H%.1f Z'
                % (x0, top, x1 + r, x1, top, x1, top + r, top + h - r,
                   x1, top + h, x1 + r, top + h, x0))
    if y1 <= y0:
        return ('M%.1f %.1f V%.1f Q%.1f %.1f %.1f %.1f H%.1f Q%.1f %.1f %.1f %.1f V%.1f Z'
                % (x, y0, y1 + r, x, y1, x + r, y1, x + w - r,
                   x + w, y1, x + w, y1 + r, y0))
    return ('M%.1f %.1f V%.1f Q%.1f %.1f %.1f %.1f H%.1f Q%.1f %.1f %.1f %.1f V%.1f Z'
            % (x, y0, y1 - r, x, y1, x + r, y1, x + w - r,
               x + w, y1, x + w, y1 - r, y0))


@chart("Bar")
def bar_chart(labels, values, title=None, series=None, horizontal=False, *, width, height):
    """Bars for each label: one bar, or with values a list of lists, one for
    each series side by side. `series` names the series for the legend."""
    labels = [_label(l) for l in _list(labels, 'labels')]
    lists, names = _series(values, series)
    for v in lists:
        if len(v) != len(labels):
            raise ValueError('there are %d values for %d labels' % (len(v), len(labels)))
    svg, (left, top, right, bottom) = _frame(width, height, title, names)
    every = [x for v in lists for x in v] or [0.0]
    ticks = _nice(min(0.0, min(every)), max(0.0, max(every)))
    if horizontal:
        left += max([_text_width(l, 9) for l in labels] or [0]) + 6
        bottom -= 14
    else:
        left += max(_text_width(_tick_text(t), 9) for t in ticks) + 6
        bottom -= 14
    place = _value_axis(svg, ticks, left, top, right, bottom, horizontal)
    n = max(len(labels), 1)
    span = (bottom - top) if horizontal else (right - left)
    band = span / n
    group = min(band * 0.7, 24.0 * len(lists) + 2 * (len(lists) - 1))
    bar = (group - 2 * (len(lists) - 1)) / len(lists)
    zero = place(0.0)
    for i, label in enumerate(labels):
        start = (top if horizontal else left) + band * i + (band - group) / 2
        for k, v in enumerate(lists):
            x = start + k * (bar + 2)
            svg.add('<path d="%s" fill="%s"/>'
                    % (_bar_path(x, zero, place(v[i]), bar, horizontal), _SERIES[k]))
        mid = (top if horizontal else left) + band * (i + 0.5)
        if horizontal:
            svg.text(left - 6, mid + 3, label, anchor='end')
        elif _text_width(label, 9) <= band or n <= 12:
            svg.text(mid, bottom + 12, label, anchor='middle')
    # The baseline, at zero.
    line = (zero, top, zero, bottom) if horizontal else (left, zero, right, zero)
    svg.add('<line x1="%.1f" y1="%.1f" x2="%.1f" y2="%.1f" stroke="%s" stroke-width="1"/>'
            % (line + (_MUTED,)))
    return svg.done()


@chart("Line")
def line_chart(labels, values, title=None, series=None, *, width, height):
    """A line through the values at each label (dates or text, in order):
    one line, or with values a list of lists, one for each series."""
    labels = [_label(l) for l in _list(labels, 'labels')]
    lists, names = _series(values, series)
    for v in lists:
        if len(v) != len(labels):
            raise ValueError('there are %d values for %d labels' % (len(v), len(labels)))
    svg, (left, top, right, bottom) = _frame(width, height, title, names)
    every = [x for v in lists for x in v] or [0.0]
    lo, hi = min(every), max(every)
    ticks = _nice(lo if lo < 0 or lo > hi * 0.5 else 0.0, hi)
    left += max(_text_width(_tick_text(t), 9) for t in ticks) + 6
    bottom -= 14
    place = _value_axis(svg, ticks, left, top, right, bottom)
    n = len(labels)
    # Room for half of the first and the last label, and the end marker.
    widest = max([_text_width(l, 9) for l in labels] or [0])
    inner_left = left + max(6.0, min(widest, _text_width(labels[0], 9) if labels else 0) / 2)
    inner_right = right - max(6.0, _text_width(labels[-1], 9) / 2 if labels else 0)
    step = (inner_right - inner_left) / max(n - 1, 1)

    def x_of(i):
        return inner_left + step * i if n > 1 else (left + right) / 2

    shown = max(1, int(_math.ceil(max([_text_width(l, 9) for l in labels] or [1]) / max(step, 1) * 1.3)))
    for i, label in enumerate(labels):
        if i % shown == 0:
            svg.text(x_of(i), bottom + 12, label, anchor='middle')
    for k, v in enumerate(lists):
        points = ' '.join('%.1f,%.1f' % (x_of(i), place(y)) for i, y in enumerate(v))
        svg.add('<polyline points="%s" fill="none" stroke="%s" stroke-width="2" '
                'stroke-linejoin="round" stroke-linecap="round"/>' % (points, _SERIES[k]))
        if v:
            svg.add('<circle cx="%.1f" cy="%.1f" r="4" fill="%s" stroke="%s" stroke-width="2"/>'
                    % (x_of(len(v) - 1), place(v[-1]), _SERIES[k], _SURFACE))
    return svg.done()


@chart("Pie")
def pie_chart(labels, values, title=None, donut=True, *, width, height):
    """A slice for each label, as its share of the total, with a legend.
    Values must not be negative."""
    labels = [_label(l) for l in _list(labels, 'labels')]
    values = [_number(v, 'values') for v in _list(values, 'values')]
    if len(values) != len(labels):
        raise ValueError('there are %d values for %d labels' % (len(values), len(labels)))
    if any(v < 0 for v in values):
        raise ValueError('a pie cannot show a negative value')
    if len(values) > len(_SERIES):
        raise ValueError('a pie shows at most %d slices' % len(_SERIES))
    svg, (left, top, right, bottom) = _frame(width, height, title, [])
    total = sum(values)
    legend_w = max([_text_width('%s  %d%%' % (l, 100), 9) for l in labels] or [0]) + 20
    r = max(4.0, min((right - left - legend_w) / 2 - 8, (bottom - top) / 2))
    cx, cy = left + r, (top + bottom) / 2
    angle = -_math.pi / 2
    for i, v in enumerate(values):
        if total <= 0 or v == 0:
            continue
        sweep = v / total * 2 * _math.pi
        if sweep >= 2 * _math.pi - 1e-9:
            svg.add('<circle cx="%.1f" cy="%.1f" r="%.1f" fill="%s"/>' % (cx, cy, r, _SERIES[i]))
        else:
            x0, y0 = cx + r * _math.cos(angle), cy + r * _math.sin(angle)
            x1, y1 = cx + r * _math.cos(angle + sweep), cy + r * _math.sin(angle + sweep)
            svg.add('<path d="M%.1f %.1f L%.1f %.1f A%.1f %.1f 0 %d 1 %.1f %.1f Z" fill="%s" '
                    'stroke="%s" stroke-width="2" stroke-linejoin="round"/>'
                    % (cx, cy, x0, y0, r, r, 1 if sweep > _math.pi else 0, x1, y1,
                       _SERIES[i], _SURFACE))
        angle += sweep
    if donut:
        svg.add('<circle cx="%.1f" cy="%.1f" r="%.1f" fill="%s"/>' % (cx, cy, r * 0.58, _SURFACE))
        svg.text(cx, cy + 4, _tick_text(round(total, 2)), size=11, anchor='middle', color=_TEXT,
                 weight='600')
    y = cy - len(labels) * 8 + 8
    x = cx + r + 16
    for i, (label, v) in enumerate(zip(labels, values)):
        share = '%d%%' % round(v / total * 100) if total > 0 else ''
        svg.add('<rect x="%.1f" y="%.1f" width="8" height="8" rx="2" fill="%s"/>'
                % (x, y - 7, _SERIES[i]))
        svg.text(x + 12, y, label, color=_TEXT)
        svg.text(x + 12 + _text_width(label, 9) + 6, y, share)
        y += 16
    return svg.done()


@chart("Scatter")
def scatter_chart(x, y, title=None, labels=None, *, width, height):
    """A dot for each pair of x and y (numbers), with its label beside it
    when labels are given."""
    xs = [_number(v, 'x') for v in _list(x, 'x')]
    ys = [_number(v, 'y') for v in _list(y, 'y')]
    if len(xs) != len(ys):
        raise ValueError('there are %d values of x for %d of y' % (len(xs), len(ys)))
    names = [_label(l) for l in _list(labels, 'labels')] if labels is not None else []
    svg, (left, top, right, bottom) = _frame(width, height, title, [])
    yt = _nice(min(ys or [0.0]), max(ys or [1.0]))
    xt = _nice(min(xs or [0.0]), max(xs or [1.0]))
    left += max(_text_width(_tick_text(t), 9) for t in yt) + 6
    bottom -= 14
    place_y = _value_axis(svg, yt, left, top, right, bottom)
    lo, hi = xt[0], xt[-1]

    def place_x(v):
        return left + (v - lo) / (hi - lo) * (right - left)

    for t in xt:
        svg.text(place_x(t), bottom + 12, _tick_text(t), anchor='middle')
    for i, (a, b) in enumerate(zip(xs, ys)):
        px, py = place_x(a), place_y(b)
        svg.add('<circle cx="%.1f" cy="%.1f" r="4" fill="%s" stroke="%s" stroke-width="2"/>'
                % (px, py, _SERIES[0], _SURFACE))
        if i < len(names) and names[i]:
            svg.text(px + 6, py - 4, names[i], size=8)
    return svg.done()
