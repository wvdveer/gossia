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
#
# The chart's Text colour and Fill colour reach the SVG through a style
# sheet the report adds: the root element's `color` is the text colour, so
# draw text and lines in `currentColor`; an element of class `background`
# is made transparent when the chart has a fill (the report paints it
# behind); one of class `surface` is filled with the fill colour and one of
# class `surface-stroke` stroked with it (holes, gaps and rings that show
# the background through).

import math as _math

# The colours of the series, in order (never cycled: a ninth series is an
# error). The rest of the chart is drawn in the text colour (lighter for
# what matters less) on the surface, for the light page of a report unless
# the chart has colours of its own.
_SERIES = ['#2a78d6', '#eb6834', '#1baf7a', '#eda100',
           '#e87ba4', '#008300', '#6250d6', '#e34948']
_SURFACE = '#fcfcfb'
_INK = '#0b0b0b'
_TEXT = 'currentColor'
_MUTED = 'muted'
_GRID = 'stroke="currentColor" stroke-opacity="0.16"'
_BASELINE = 'stroke="currentColor" stroke-opacity="0.6"'
_RING = 'stroke="%s" class="surface-stroke"' % _SURFACE
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
        fill = 'fill="currentColor" fill-opacity="0.72"' if color == _MUTED else 'fill="%s"' % color
        self.add('<text x="%.1f" y="%.1f" font-size="%g" text-anchor="%s" %s%s>%s</text>'
                 % (x, y, size, anchor, fill, w, _esc(text)))

    def label(self, x, y, name, extra, anchor='start'):
        # A name with a value after it in lighter text, with a ring of the
        # surface so it reads over what is behind it.
        self.add('<text x="%.1f" y="%.1f" font-size="9" text-anchor="%s" fill="currentColor" '
                 'stroke="%s" stroke-width="3" stroke-linejoin="round" paint-order="stroke" '
                 'class="surface-stroke">%s <tspan fill-opacity="0.72">%s</tspan></text>'
                 % (x, y, anchor, _SURFACE, _esc(name), _esc(extra)))

    def done(self):
        return ('<svg xmlns="http://www.w3.org/2000/svg" width="%g" height="%g" '
                'viewBox="0 0 %g %g" font-family="%s" color="%s">'
                % (self.width, self.height, self.width, self.height, _FONT, _INK)
                + '<rect width="100%%" height="100%%" fill="%s" class="background"/>' % _SURFACE
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
            svg.add('<line x1="%.1f" y1="%.1f" x2="%.1f" y2="%.1f" %s stroke-width="1"/>'
                    % (x, top, x, bottom, _GRID))
            svg.text(x, bottom + 11, _tick_text(t), anchor='middle')
    else:
        def place(v):
            return bottom - (v - lo) / (hi - lo) * (bottom - top)
        for t in ticks:
            y = place(t)
            svg.add('<line x1="%.1f" y1="%.1f" x2="%.1f" y2="%.1f" %s stroke-width="1"/>'
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
    svg.add('<line x1="%.1f" y1="%.1f" x2="%.1f" y2="%.1f" %s stroke-width="1"/>'
            % (line + (_BASELINE,)))
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
            svg.add('<circle cx="%.1f" cy="%.1f" r="4" fill="%s" %s stroke-width="2"/>'
                    % (x_of(len(v) - 1), place(v[-1]), _SERIES[k], _RING))
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
                    '%s stroke-width="2" stroke-linejoin="round"/>'
                    % (cx, cy, x0, y0, r, r, 1 if sweep > _math.pi else 0, x1, y1,
                       _SERIES[i], _RING))
        angle += sweep
    if donut:
        svg.add('<circle cx="%.1f" cy="%.1f" r="%.1f" fill="%s" class="surface"/>'
                % (cx, cy, r * 0.58, _SURFACE))
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
        svg.add('<circle cx="%.1f" cy="%.1f" r="4" fill="%s" %s stroke-width="2"/>'
                % (px, py, _SERIES[0], _RING))
        if i < len(names) and names[i]:
            svg.text(px + 6, py - 4, names[i], size=8)
    return svg.done()


@chart("Sankey")
def sankey_chart(sources, targets, values, title=None, *, width, height):
    """Flows from each source to its target, as bands as thick as their
    values: sources, targets and values are lists of the same length, one
    flow at each place. A name can be the target of one flow and the source
    of others; flows lead on, never round in a circle."""
    froms = [_label(s) for s in _list(sources, 'sources')]
    tos = [_label(t) for t in _list(targets, 'targets')]
    amounts = [_number(v, 'values') for v in _list(values, 'values')]
    if not len(froms) == len(tos) == len(amounts):
        raise ValueError('there are %d sources, %d targets and %d values; they go together, '
                         'one flow at each place' % (len(froms), len(tos), len(amounts)))
    names = []
    index = {}

    def node(name):
        if name not in index:
            index[name] = len(names)
            names.append(name)
        return index[name]

    flows = []
    for a, b, v in zip(froms, tos, amounts):
        if v < 0:
            raise ValueError('a flow cannot be negative: %s to %s is %s' % (a, b, _tick_text(v)))
        if a == b:
            raise ValueError('%s flows to itself' % a)
        i, j = node(a), node(b)
        if v > 0:
            flows.append((i, j, v))
    n = len(names)
    outs = [[] for _ in range(n)]
    ins = [[] for _ in range(n)]
    for k, (i, j, v) in enumerate(flows):
        outs[i].append(k)
        ins[j].append(k)

    # Columns: the longest way from a start; the ends all in the last column.
    column = [0] * n
    waiting = [len(ins[i]) for i in range(n)]
    ready = [i for i in range(n) if waiting[i] == 0]
    done = 0
    while ready:
        i = ready.pop(0)
        done += 1
        for k in outs[i]:
            j = flows[k][1]
            column[j] = max(column[j], column[i] + 1)
            waiting[j] -= 1
            if waiting[j] == 0:
                ready.append(j)
    if done < n:
        raise ValueError('the flows go round in a circle; a Sankey diagram shows flows that '
                         'lead on, never back')
    last = max(column or [0])
    for i in range(n):
        if ins[i] and not outs[i]:
            column[i] = last
    value = [max(sum(flows[k][2] for k in ins[i]), sum(flows[k][2] for k in outs[i]))
             for i in range(n)]

    svg, (left, top, right, bottom) = _frame(width, height, title, [])
    bar = 6.0
    span = bottom - top
    columns = [[i for i in range(n) if column[i] == c] for c in range(last + 1)]
    gap = min([8.0] + [span * 0.4 / (len(c) - 1) for c in columns if len(c) > 1])
    scales = [(span - gap * (len(c) - 1)) / sum(value[i] for i in c)
              for c in columns if c and sum(value[i] for i in c) > 0]
    ky = max(0.0, min(scales)) if scales else 0.0

    def x_of(c):
        return left + (right - left - bar) * c / last if last else left

    y0 = [0.0] * n
    sy = {}
    ty = {}

    def stack():
        for members in columns:
            used = sum(value[i] * ky for i in members) + gap * max(len(members) - 1, 0)
            y = top + (span - used) / 2
            for i in members:
                y0[i] = y
                y += value[i] * ky + gap
        # The bands, stacked at each end in the order of the other end.
        for i in range(n):
            outs[i].sort(key=lambda k: (y0[flows[k][1]], flows[k][1]))
            ins[i].sort(key=lambda k: (y0[flows[k][0]], flows[k][0]))
            at = y0[i]
            for k in outs[i]:
                sy[k] = at
                at += flows[k][2] * ky
            at = y0[i]
            for k in ins[i]:
                ty[k] = at
                at += flows[k][2] * ky

    def middle(i, ends, mine):
        # Where the bands of a node meet the other column, on average.
        total = sum(flows[k][2] for k in ends)
        if not total:
            return y0[i] + value[i] * ky / 2
        return sum((mine[k] + flows[k][2] * ky / 2) * flows[k][2] for k in ends) / total

    # In order of the flows' places at their other end, a few times over:
    # fewer bands cross.
    stack()
    for _ in range(4):
        for c in range(1, last + 1):
            columns[c].sort(key=lambda i: middle(i, ins[i], sy))
        stack()
        for c in range(last - 1, -1, -1):
            columns[c].sort(key=lambda i: middle(i, outs[i], ty))
        stack()

    # A colour for each name that flows on, in order, when there are few
    # enough; otherwise the flows are in the text colour.
    starts = [i for i in range(n) if outs[i]]
    colour = {}
    if len(starts) <= len(_SERIES):
        for pos, i in enumerate(starts):
            colour[i] = _SERIES[pos]

    def paint(i, opacity):
        if i in colour:
            return 'fill="%s" fill-opacity="%g"' % (colour[i], opacity)
        return 'fill="currentColor" fill-opacity="%g"' % (opacity * 0.5)

    for k, (i, j, v) in enumerate(flows):
        t = v * ky
        x0 = x_of(column[i]) + bar
        x1 = x_of(column[j])
        xm = (x0 + x1) / 2
        a, b = sy[k], ty[k]
        svg.add('<path d="M%.1f %.1f C%.1f %.1f %.1f %.1f %.1f %.1f L%.1f %.1f '
                'C%.1f %.1f %.1f %.1f %.1f %.1f Z" %s/>'
                % (x0, a, xm, a, xm, b, x1, b, x1, b + t, xm, b + t, xm, a + t, x0, a + t,
                   paint(i, 0.38)))
    for i in range(n):
        h = max(value[i] * ky, 1.0)
        svg.add('<rect x="%.1f" y="%.1f" width="%g" height="%.1f" rx="1" %s/>'
                % (x_of(column[i]), y0[i], bar, h, paint(i, 1)))
    # A label beside each node with room for it: on the right, but the last
    # column on the left.
    for i in range(n):
        h = value[i] * ky
        if h + gap < 9:
            continue
        x = x_of(column[i])
        end = column[i] == last and last > 0
        svg.label(x - 4 if end else x + bar + 4, y0[i] + h / 2 + 3, names[i],
                  _tick_text(round(value[i], 2)), 'end' if end else 'start')
    return svg.done()
