# Gossia pyplot (DESIGN.md, "Gossia pyplot, 0.6.0"): Gossia's own module for
# drawing a chart as SVG, available as `plt`. Its function names and
# arguments follow matplotlib's pyplot where they overlap, but it is not
# matplotlib, which cannot run in Gossia; what it does not cover is refused
# by name.
#
# A chart function starts with plt.clear() and ends with return plt.svg():
#
#     @chart("Sales")
#     def sales(months, totals, *, width, height):
#         plt.clear()
#         plt.plot(months, totals, label="Sales")
#         plt.title("Sales")
#         return plt.svg()
#
# The figure is the size of the chart in the report unless plt.figure() is
# given another, in points. Text and axes are drawn in the chart's text
# colour (`currentColor`), on a background of class `background`, so a
# chart follows the report's colours and the window's light or dark theme.

import math as _math
import datetime as _dt
from _gossia import text_width as _text_em

_FONT = 'Inter'
_SURFACE = '#fcfcfb'
_INK = '#0b0b0b'
_TICK_SIZE = 9.0
_LABEL_SIZE = 10.0
_TITLE_SIZE = 12.0
_SUPTITLE_SIZE = 13.0
_PAD = 6.0
_TICK_LEN = 3.5
_TICK_GAP = 2.5
_ASCENT = 0.73
_LINE = 1.25
_MINUS = '−'

# matplotlib's colours of the series, in order, cycled.
_CYCLE = ['#1f77b4', '#ff7f0e', '#2ca02c', '#d62728', '#9467bd',
          '#8c564b', '#e377c2', '#7f7f7f', '#bcbd22', '#17becf']
_TAB = dict(zip(['tab:blue', 'tab:orange', 'tab:green', 'tab:red', 'tab:purple',
                 'tab:brown', 'tab:pink', 'tab:gray', 'tab:olive', 'tab:cyan'], _CYCLE))
_TAB['tab:grey'] = _TAB['tab:gray']
_LETTERS = {'b': '#0000ff', 'g': '#008000', 'r': '#ff0000', 'c': '#00bfbf',
            'm': '#bf00bf', 'y': '#bfbf00', 'k': '#000000', 'w': '#ffffff'}
_LINESTYLES = {'-': None, 'solid': None, '--': '5.55,2.4', 'dashed': '5.55,2.4',
               '-.': '9.6,2.4,1.5,2.4', 'dashdot': '9.6,2.4,1.5,2.4',
               ':': '1.5,2.475', 'dotted': '1.5,2.475', 'None': 'none',
               'none': 'none', '': 'none', ' ': 'none'}
_MARKERS = '.,ov^<>sp*hHxXD+d|_'

# Keyword arguments that make no difference to an SVG at the chart's size,
# accepted and ignored so that code from elsewhere does not fail on them.
_IGNORED = frozenset(['dpi', 'zorder', 'antialiased', 'rasterized', 'picker',
                      'clip_on', 'snap'])

_range = range

_state = {'size': (400.0, 300.0), 'figures': [], 'current': None}


# ---------------------------------------------------------------- helpers


def _refuse(what):
    if what.split('.')[-1] == 'show':
        # There is no show(): a chart function gives its SVG back.
        return AttributeError('gossia.pyplot has no show(): finish the chart function'
                              ' with return plt.svg()')
    return AttributeError('gossia.pyplot does not support %s' % what)


def _check(kwargs, allowed, where):
    """Drop the ignored arguments; refuse any not in `allowed`."""
    for key in list(kwargs):
        if key in _IGNORED:
            del kwargs[key]
        elif key not in allowed:
            raise TypeError('gossia.pyplot does not support the argument %s of %s'
                            % (key, where))
    return kwargs


def _esc(text):
    return (str(text).replace('&', '&amp;').replace('<', '&lt;')
            .replace('>', '&gt;').replace('"', '&quot;'))


def _width(text, size, bold=False):
    return _text_em(str(text)) * size * (1.06 if bold else 1.0)


def _fmt(v):
    text = '%.2f' % v
    text = text.rstrip('0').rstrip('.')
    return '0' if text == '-0' else text


def _list(values, what):
    values = PLAIN(values)
    if isinstance(values, (str, bytes, dict)):
        raise TypeError('%s must be a list' % what)
    try:
        return list(values)
    except TypeError:
        raise TypeError('%s must be a list, not %s' % (what, type(values).__name__)) from None


def _colour(value, what='color'):
    """A colour as SVG takes it: matplotlib's names, `C0`, hex, grey levels as
    text and (r, g, b) tuples."""
    if value is None:
        return None
    if isinstance(value, (tuple, list)):
        parts = [float(p) for p in value]
        if len(parts) not in (3, 4) or not all(0.0 <= p <= 1.0 for p in parts):
            raise ValueError('a %s as numbers is (red, green, blue) from 0 to 1' % what)
        return '#%02x%02x%02x' % tuple(int(round(p * 255)) for p in parts[:3])
    if not isinstance(value, str):
        raise TypeError('a %s is text, such as "tab:blue" or "#1f77b4"' % what)
    text = value.strip()
    if text in _LETTERS:
        return _LETTERS[text]
    if text.lower() in _TAB:
        return _TAB[text.lower()]
    if len(text) == 2 and text[0] == 'C' and text[1].isdigit():
        return _CYCLE[int(text[1])]
    if text.startswith('#') and len(text) in (4, 7, 9):
        return text[:7] if len(text) == 9 else text
    try:
        grey = float(text)
    except ValueError:
        grey = None
    if grey is not None:
        if not 0.0 <= grey <= 1.0:
            raise ValueError('a grey %s is a number from 0 to 1, as text' % what)
        level = int(round(grey * 255))
        return '#%02x%02x%02x' % (level, level, level)
    if text.replace(' ', '').isalpha():
        # The CSS names, which SVG knows: "red", "steelblue".
        return text.replace(' ', '').lower()
    raise ValueError('%r is not a colour' % value)


def _superscript(n):
    digits = '⁰¹²³⁴⁵⁶⁷⁸⁹'
    text = ''.join(digits[int(d)] for d in str(abs(n)))
    return ('⁻' if n < 0 else '') + text


def _fnv(text):
    h = 0xcbf29ce484222325
    for ch in text:
        h = ((h ^ ord(ch)) * 0x100000001b3) & 0xffffffffffffffff
    return '%012x' % (h >> 16)


# ------------------------------------------------------------------- axes


class _Axis:
    """One axis of an Axes: what its values are (numbers, dates or
    categories), its scale, and what the user set."""

    def __init__(self, name, owner):
        self.name = name
        self.owners = [owner]     # the Axes that use this axis
        self.kind = None          # None, 'number', 'date', 'category'
        self.datetime = False     # dates with times
        self.categories = {}      # category -> position
        self.scale = 'linear'
        self.limits = [None, None]
        self.ticks = None         # positions set by the user
        self.tick_labels = None
        self.rotation = 0.0
        self.label = None
        self.label_size = _LABEL_SIZE
        self.tick_size = _TICK_SIZE
        self.inverted = False
        self.tight = False

    def _kind(self, kind):
        if self.kind is None:
            self.kind = kind
        elif self.kind != kind:
            names = {'number': 'numbers', 'date': 'dates', 'category': 'text'}
            raise TypeError('the %s axis has %s, so it cannot take %s'
                            % (self.name, names[self.kind], names[kind]))

    def convert(self, values, what):
        """The values as positions on this axis: numbers, dates as days, or
        text as categories; None (and NaN) for a gap."""
        values = _list(values, what)
        out = []
        for v in values:
            if v is None:
                out.append(None)
            elif isinstance(v, bool):
                raise TypeError('%s cannot be True or False' % what)
            elif isinstance(v, _dt.date):
                self._kind('date')
                out.append(_date_number(v, self))
            elif isinstance(v, str):
                self._kind('category')
                if v not in self.categories:
                    self.categories[v] = float(len(self.categories))
                out.append(self.categories[v])
            else:
                try:
                    f = float(v)
                except (TypeError, ValueError):
                    raise TypeError('%s must be numbers, dates or text, not %r'
                                    % (what, v)) from None
                self._kind('number')
                out.append(None if f != f else f)
        return out

    def one(self, value, what):
        if value is None:
            return None
        return self.convert([value], what)[0]


def _date_number(value, axis):
    if isinstance(value, _dt.datetime):
        axis.datetime = True
        seconds = value.hour * 3600 + value.minute * 60 + value.second + value.microsecond / 1e6
        return value.toordinal() + seconds / 86400.0
    return float(value.toordinal())


def _number_date(number):
    day = int(_math.floor(number))
    seconds = round((number - day) * 86400.0)
    return _dt.datetime.fromordinal(max(day, 1)) + _dt.timedelta(seconds=seconds)


class _Handle:
    """Something drawn that the user may hold: a line, a bar, a wedge, a
    text. Its look is a dict, `style`, which the set_ methods change."""

    _kind = 'patch'

    def __init__(self, style):
        self.style = style

    def __getattr__(self, name):
        raise _refuse('%s.%s' % (type(self).__name__.lstrip('_'), name))

    def get_label(self):
        return self.style.get('label') or ''

    def set_label(self, label):
        self.style['label'] = None if label is None else str(label)

    def set_color(self, color):
        c = _colour(color)
        self.style['color'] = c
        if 'facecolor' in self.style:
            self.style['facecolor'] = c

    def get_color(self):
        return self.style.get('color') or self.style.get('facecolor')

    def set_facecolor(self, color):
        self.style['facecolor'] = _colour(color)

    set_fc = set_facecolor

    def get_facecolor(self):
        return self.style.get('facecolor')

    def set_edgecolor(self, color):
        self.style['edgecolor'] = _colour(color)

    set_ec = set_edgecolor

    def set_linewidth(self, width):
        self.style['linewidth'] = float(width)

    set_lw = set_linewidth

    def set_linestyle(self, line):
        self.style['dash'] = _dash(line)

    set_ls = set_linestyle

    def set_alpha(self, alpha):
        self.style['alpha'] = None if alpha is None else float(alpha)

    def set_visible(self, visible):
        self.style['visible'] = bool(visible)

    def set_marker(self, marker):
        self.style['marker'] = _marker_name(marker)

    def set_markersize(self, size):
        self.style['markersize'] = float(size)

    set_ms = set_markersize

    def set_fontsize(self, size):
        self.style['fontsize'] = float(size)

    set_size = set_fontsize

    def set_fontweight(self, weight):
        self.style['fontweight'] = weight

    set_weight = set_fontweight

    def set_text(self, text):
        self.style['text'] = str(text)

    def get_text(self):
        return self.style.get('text', '')

    def set(self, **kwargs):
        for key, value in kwargs.items():
            setter = getattr(type(self), 'set_' + key, None)
            if setter is None:
                raise _refuse('%s.set(%s=...)' % (type(self).__name__.lstrip('_'), key))
            setter(self, value)


class _Artist:
    """What an Axes draws. bounds() gives the data's extent along x and y
    (either may be None), sticky() the values a margin may not pass,
    points() where it lies in the SVG (for placing a legend), and entries()
    the handles it shows in a legend."""

    container = False

    def bounds(self, axes):
        return None

    def sticky(self):
        return [], []

    def points(self, axes, box):
        return []

    def entries(self):
        return []

    def draw(self, out, axes, box):
        pass

    def draw_over(self, out, axes, box):
        """What is drawn outside the clip: texts."""
        pass


def _opacity(style, attr='opacity'):
    alpha = style.get('alpha')
    return ' %s="%s"' % (attr, _fmt(alpha)) if alpha is not None else ''


def _dash(line):
    if line is None:
        return None
    if isinstance(line, (tuple, list)):
        # (offset, (on, off, ...)) as matplotlib takes it.
        if len(line) == 2 and isinstance(line[1], (tuple, list)):
            return ','.join(_fmt(float(v) * 1.5) for v in line[1])
        raise ValueError('a line style is "-", "--", "-.", ":" or (offset, (on, off))')
    if line not in _LINESTYLES:
        raise ValueError('%r is not a line style' % (line,))
    return _LINESTYLES[line]


def _dash_attr(dash, width):
    if not dash or dash == 'none':
        return ''
    return ' stroke-dasharray="%s"' % ','.join(
        _fmt(float(d) * width / 1.5) for d in dash.split(','))


def _marker_name(marker):
    if marker in (None, '', 'None', 'none', ' '):
        return None
    if not isinstance(marker, str) or marker not in _MARKERS + '1234':
        raise ValueError('gossia.pyplot does not support the marker %r' % (marker,))
    return marker


def _runs(xs, ys, axes, box):
    """The points placed in the SVG, split where one cannot be drawn."""
    points = []
    runs = []
    for x, y in zip(xs, ys):
        p = axes._place(x, y, box)
        if p is None:
            if points:
                runs.append(points)
            points = []
        else:
            points.append(p)
    if points:
        runs.append(points)
    return runs


def _steps(run, where):
    """A run of points as steps: 'pre', 'post' or 'mid'."""
    if len(run) < 2:
        return run
    out = [run[0]]
    for (x0, y0), (x1, y1) in zip(run, run[1:]):
        if where == 'pre':
            out += [(x0, y1), (x1, y1)]
        elif where == 'post':
            out += [(x1, y0), (x1, y1)]
        else:
            m = (x0 + x1) / 2.0
            out += [(m, y0), (m, y1), (x1, y1)]
    return out


def _path(run):
    return 'M' + ' L'.join('%s %s' % (_fmt(px), _fmt(py)) for px, py in run)


def _sample(run):
    """Points along a run of segments, for placing a legend."""
    out = list(run)
    for (x0, y0), (x1, y1) in zip(run, run[1:]):
        for i in range(1, 6):
            t = i / 6.0
            out.append((x0 + (x1 - x0) * t, y0 + (y1 - y0) * t))
    return out


class _Line(_Handle, _Artist):
    _kind = 'line'

    def __init__(self, xs, ys, style):
        _Handle.__init__(self, style)
        self.xs = xs
        self.ys = ys

    def bounds(self, axes):
        return _points_bounds(self.xs, self.ys, axes)

    def entries(self):
        return [self]

    def get_xdata(self):
        return list(self.xs)

    def get_ydata(self):
        return list(self.ys)

    def _placed(self, axes, box):
        runs = _runs(self.xs, self.ys, axes, box)
        steps = self.style.get('steps')
        return runs, ([_steps(r, steps) for r in runs] if steps else runs)

    def points(self, axes, box):
        out = []
        for run in self._placed(axes, box)[1]:
            out += _sample(run)
        return out

    def draw(self, out, axes, box):
        style = self.style
        if style.get('visible') is False:
            return
        color = style['color']
        runs, paths = self._placed(axes, box)
        width = style.get('linewidth', 1.5)
        opacity = _opacity(style)
        dash = style.get('dash')
        if dash != 'none':
            for run in paths:
                if len(run) < 2:
                    continue
                out.append('<path d="%s" fill="none" stroke="%s" stroke-width="%s"'
                           ' stroke-linejoin="round" stroke-linecap="butt"%s%s/>'
                           % (_path(run), color, _fmt(width), _dash_attr(dash, width), opacity))
        marker = style.get('marker')
        if marker:
            size = style.get('markersize', 6.0)
            face = style.get('markerfacecolor') or color
            edge = style.get('markeredgecolor') or color
            for run in runs:
                for px, py in run:
                    out.append(_marker(marker, px, py, size, face, edge, opacity,
                                       style.get('markeredgewidth', 1.0)))


def _marker(marker, x, y, size, face, edge, opacity='', edge_width=1.0):
    r = size / 2.0
    common = ' fill="%s" stroke="%s" stroke-width="%s"%s' % (face, edge, _fmt(edge_width), opacity)
    if marker in ('o', '.', ','):
        if marker == '.':
            r *= 0.5
        elif marker == ',':
            r = 0.5
        return '<circle cx="%s" cy="%s" r="%s"%s/>' % (_fmt(x), _fmt(y), _fmt(r), common)
    if marker == 's':
        return '<rect x="%s" y="%s" width="%s" height="%s"%s/>' % (
            _fmt(x - r), _fmt(y - r), _fmt(2 * r), _fmt(2 * r), common)
    shapes = {
        '^': [(0, -1), (1, 1), (-1, 1)], 'v': [(0, 1), (1, -1), (-1, -1)],
        '<': [(-1, 0), (1, -1), (1, 1)], '>': [(1, 0), (-1, -1), (-1, 1)],
        'D': [(0, -1), (1, 0), (0, 1), (-1, 0)], 'd': [(0, -1), (0.6, 0), (0, 1), (-0.6, 0)],
        'p': [(_math.sin(a), -_math.cos(a)) for a in [i * 2 * _math.pi / 5 for i in range(5)]],
        'h': [(_math.sin(a), -_math.cos(a)) for a in [i * 2 * _math.pi / 6 for i in range(6)]],
        'H': [(_math.cos(a), _math.sin(a)) for a in [i * 2 * _math.pi / 6 for i in range(6)]],
        '*': [((1.0 if i % 2 == 0 else 0.4) * _math.sin(i * _math.pi / 5),
               -(1.0 if i % 2 == 0 else 0.4) * _math.cos(i * _math.pi / 5)) for i in range(10)],
    }
    if marker in shapes:
        pts = ' '.join('%s,%s' % (_fmt(x + px * r), _fmt(y + py * r)) for px, py in shapes[marker])
        return '<polygon points="%s"%s/>' % (pts, common)
    lines = {'+': [(-1, 0, 1, 0), (0, -1, 0, 1)], 'x': [(-1, -1, 1, 1), (-1, 1, 1, -1)],
             'X': [(-1, -1, 1, 1), (-1, 1, 1, -1)], '|': [(0, -1, 0, 1)], '_': [(-1, 0, 1, 0)],
             '1': [(0, 0, 0, 1), (0, 0, -0.87, -0.5), (0, 0, 0.87, -0.5)],
             '2': [(0, 0, 0, -1), (0, 0, -0.87, 0.5), (0, 0, 0.87, 0.5)],
             '3': [(0, 0, -1, 0), (0, 0, 0.5, -0.87), (0, 0, 0.5, 0.87)],
             '4': [(0, 0, 1, 0), (0, 0, -0.5, -0.87), (0, 0, -0.5, 0.87)]}
    width = 2.0 if marker == 'X' else max(1.0, edge_width)
    d = ' '.join('M%s %s L%s %s' % (_fmt(x + a * r), _fmt(y + b * r), _fmt(x + c * r), _fmt(y + e * r))
                 for a, b, c, e in lines[marker])
    # A marker made of lines takes the face colour, as it has no face.
    colour = edge if face in ('none', None) else face
    return '<path d="%s" fill="none" stroke="%s" stroke-width="%s"%s/>' % (d, colour, _fmt(width), opacity)


def _points_bounds(xs, ys, axes):
    px = [x for x, y in zip(xs, ys) if x is not None and y is not None
          and axes._usable(axes.xaxis, x) and axes._usable(axes.yaxis, y)]
    py = [y for x, y in zip(xs, ys) if x is not None and y is not None
          and axes._usable(axes.xaxis, x) and axes._usable(axes.yaxis, y)]
    if not px:
        return None
    return (min(px), max(px)), (min(py), max(py))


def _parse_format(fmt):
    """A format string such as "r--" or "o-": colour, marker and line style."""
    color = marker = None
    line = None
    rest = fmt
    for style in ('--', '-.', '-', ':'):
        if style in rest:
            line = style
            rest = rest.replace(style, '', 1)
            break
    for ch in rest:
        if ch in _LETTERS and color is None:
            color = ch
        elif ch in _MARKERS + '1234' and marker is None:
            marker = ch
        elif len(rest) == 2 and rest[0] == 'C' and rest[1].isdigit():
            color = rest
            break
        else:
            raise ValueError('%r is not a format string (colour, marker, line style)' % fmt)
    if line is None and marker is not None:
        line = 'None'
    return color, marker, line


# ---------------------------------------------------------------- artists

_LEGEND_CODES = ['best', 'upper right', 'upper left', 'lower left', 'lower right', 'right',
                 'center left', 'center right', 'lower center', 'upper center', 'center']
_FONT_SIZES = {'xx-small': 5.79, 'x-small': 6.94, 'small': 8.33, 'medium': 10.0,
               'large': 12.0, 'x-large': 14.4, 'xx-large': 17.28, 'larger': 12.0,
               'smaller': 8.33}


def _font_size(value, default):
    if value is None:
        return default
    return float(_FONT_SIZES.get(value, value))


def _share(ax, name, axis):
    """Make `ax` use `axis` for its x or y axis."""
    old = getattr(ax, name)
    if ax in old.owners:
        old.owners.remove(ax)
    setattr(ax, name, axis)
    if ax not in axis.owners:
        axis.owners.append(ax)


def _per_point(value, n, what, conv, default):
    """One value for each of n points: given once for all, or as a list."""
    value = PLAIN(value)
    if value is None:
        return [default] * n
    if isinstance(value, (list, tuple)):
        if len(value) != n:
            raise ValueError('there are %d %s for %d values' % (len(value), what, n))
        return [None if v is None else conv(v) for v in value]
    return [conv(value)] * n


def _is_rgb(value):
    return (isinstance(value, (tuple, list)) and len(value) in (3, 4)
            and all(isinstance(v, (int, float)) and not isinstance(v, bool) for v in value))


def _point_colours(value, n):
    """A colour for each of n points, bars or wedges: one for all, or a
    list (repeated when shorter)."""
    value = PLAIN(value)
    if isinstance(value, str) or (_is_rgb(value) and n != len(value)):
        return [_colour(value)] * n
    if isinstance(value, (list, tuple)):
        if value and all(isinstance(v, (int, float)) and not isinstance(v, bool) for v in value):
            raise TypeError('gossia.pyplot does not support colour maps: give the colours'
                            ' themselves, such as "tab:blue"')
        if not value:
            raise ValueError('the list of colours is empty')
        given = [_colour(v) for v in value]
        return [given[i % len(given)] for i in range(n)]
    return [_colour(value)] * n


def _pixels(axes, box, x0, x1, y0, y1):
    """A rectangle in data coordinates as (left, top, width, height) in the
    SVG; a side at or below 0 on a logarithmic axis goes to the edge."""
    a = axes._pos(axes.xaxis, x0, box, True)
    b = axes._pos(axes.xaxis, x1, box, True)
    c = axes._pos(axes.yaxis, y0, box, True)
    d = axes._pos(axes.yaxis, y1, box, True)
    if None in (a, b, c, d):
        return None
    return min(a, b), min(c, d), abs(b - a), abs(d - c)


def _stroke(style, default_width=1.0):
    edge = style.get('edgecolor')
    width = style.get('linewidth', default_width)
    if not edge or edge == 'none' or not width:
        return ' stroke="none"'
    return ' stroke="%s" stroke-width="%s"%s' % (edge, _fmt(width), _dash_attr(style.get('dash'), width))


def _segments(out, segments, colour, width, opacity='', dash=None):
    if not segments:
        return
    d = ' '.join('M%s %s L%s %s' % (_fmt(a), _fmt(b), _fmt(c), _fmt(e)) for a, b, c, e in segments)
    out.append('<path d="%s" fill="none" stroke="%s" stroke-width="%s"%s%s/>'
               % (d, colour, _fmt(width), _dash_attr(dash, width), opacity))


class _Scatter(_Handle, _Artist):
    _kind = 'marker'

    def __init__(self, xs, ys, sizes, colours, edges, style):
        _Handle.__init__(self, style)
        self.xs, self.ys = xs, ys
        self.sizes, self.colours, self.edges = sizes, colours, edges

    def bounds(self, axes):
        return _points_bounds(self.xs, self.ys, axes)

    def entries(self):
        return [self]

    def points(self, axes, box):
        return [p for p in (axes._place(x, y, box) for x, y in zip(self.xs, self.ys)) if p]

    def set_color(self, color):
        self.colours = [_colour(color)] * len(self.xs)

    set_facecolor = set_color

    def _look(self):
        size = _math.sqrt(self.sizes[0]) if self.sizes and self.sizes[0] else 6.0
        face = self.colours[0] if self.colours else _CYCLE[0]
        return {'kind': 'marker', 'marker': self.style['marker'],
                'markersize': min(max(size, 3.0), 12.0), 'markerfacecolor': face,
                'markeredgecolor': self.edges[0] if self.edges else face,
                'alpha': self.style.get('alpha')}

    def draw(self, out, axes, box):
        if self.style.get('visible') is False:
            return
        opacity = _opacity(self.style)
        for i, (x, y) in enumerate(zip(self.xs, self.ys)):
            p = axes._place(x, y, box)
            if p is None or self.sizes[i] is None:
                continue
            face = self.colours[i]
            edge = self.edges[i] if self.edges else face
            out.append(_marker(self.style['marker'], p[0], p[1], _math.sqrt(max(self.sizes[i], 0.0)),
                               face, edge, opacity, self.style['linewidth']))


class _Bar(_Handle):
    def __init__(self, style, start, thickness, base, length, vertical):
        _Handle.__init__(self, style)
        self.start, self.thickness = start, thickness
        self.base, self.length = base, length
        self.vertical = vertical

    def _rect(self):
        """(x0, x1, y0, y1) in data coordinates."""
        a, b = self.start, self.start + self.thickness
        c, d = self.base, self.base + self.length
        return (a, b, c, d) if self.vertical else (c, d, a, b)

    def get_x(self):
        return self._rect()[0]

    def get_y(self):
        return self._rect()[2]

    def get_width(self):
        r = self._rect()
        return r[1] - r[0]

    def get_height(self):
        r = self._rect()
        return r[3] - r[2]

    def set_height(self, h):
        if self.vertical:
            self.length = float(h)
        else:
            self.thickness = float(h)

    def set_width(self, w):
        if self.vertical:
            self.thickness = float(w)
        else:
            self.length = float(w)

    def set_color(self, color):
        self.style['facecolor'] = _colour(color)
        self.style['edgecolor'] = None


class _Bars(_Artist):
    def __init__(self, patches, vertical):
        self.patches = patches
        self.vertical = vertical
        self.entry = None

    def entries(self):
        return [self.entry] if self.entry is not None else []

    def bounds(self, axes):
        xs, ys = [], []
        for p in self.patches:
            x0, x1, y0, y1 = p._rect()
            xs += [v for v in (x0, x1) if axes._usable(axes.xaxis, v)]
            ys += [v for v in (y0, y1) if axes._usable(axes.yaxis, v)]
        if not xs or not ys:
            return None
        return (min(xs), max(xs)), (min(ys), max(ys))

    def sticky(self):
        bases = [p.base for p in self.patches]
        return ([], bases) if self.vertical else (bases, [])

    def points(self, axes, box):
        out = []
        for p in self.patches:
            r = _pixels(axes, box, *p._rect())
            if r:
                x, y, w, h = r
                out += [(x + w * i / 2.0, y + h * j / 2.0) for i in range(3) for j in range(3)]
        return out

    def draw(self, out, axes, box):
        for p in self.patches:
            if p.style.get('visible') is False:
                continue
            r = _pixels(axes, box, *p._rect())
            if r is None:
                continue
            x, y, w, h = r
            s = p.style
            out.append('<rect x="%s" y="%s" width="%s" height="%s" fill="%s"%s%s/>'
                       % (_fmt(x), _fmt(y), _fmt(w), _fmt(h), s.get('facecolor') or 'none',
                          _stroke(s), _opacity(s)))


class _BarContainer(list):
    """What bar() and barh() give: the bars, as a list, with the label."""

    def __init__(self, patches, label):
        list.__init__(self, patches)
        self.patches = list(patches)
        self._label = None if label is None else str(label)

    def get_label(self):
        return self._label or ''

    def set_label(self, label):
        self._label = None if label is None else str(label)

    def _look(self):
        if not self.patches:
            return None
        return dict(self.patches[0].style, kind='patch')


def _errors(err, n, what, keep=None):
    """Error sizes as (below, above) lists: one size for all, one per point,
    or [below, above] lists."""
    err = PLAIN(err)
    if err is None:
        return None
    if isinstance(err, (list, tuple)):
        if len(err) == 2 and all(isinstance(e, (list, tuple)) for e in err):
            lo = [float(v) for v in err[0]]
            hi = [float(v) for v in err[1]]
        else:
            lo = hi = [float(v) for v in err]
    else:
        lo = hi = [float(err)] * n
    if len(lo) != n or len(hi) != n:
        raise ValueError('there are %d %s values for %d points' % (len(lo), what, n))
    if any(v < 0 for v in lo + hi):
        raise ValueError('%s cannot be negative' % what)
    if keep is not None:
        lo = [v if k else None for v, k in zip(lo, keep)]
        hi = [v if k else None for v, k in zip(hi, keep)]
    return lo, hi


class _ErrorBars(_Artist):
    def __init__(self, xs, ys, xerr, yerr, style):
        self.xs, self.ys = xs, ys
        self.xerr, self.yerr = xerr, yerr
        self.style = style

    def _ends(self):
        """Each error bar as ((x0, y0), (x1, y1), horizontal)."""
        out = []
        for i, (x, y) in enumerate(zip(self.xs, self.ys)):
            if x is None or y is None:
                continue
            if self.yerr and self.yerr[0][i] is not None:
                out.append(((x, y - self.yerr[0][i]), (x, y + self.yerr[1][i]), False))
            if self.xerr and self.xerr[0][i] is not None:
                out.append(((x - self.xerr[0][i], y), (x + self.xerr[1][i], y), True))
        return out

    def bounds(self, axes):
        xs, ys = [], []
        for a, b, _ in self._ends():
            for x, y in (a, b):
                if axes._usable(axes.xaxis, x) and axes._usable(axes.yaxis, y):
                    xs.append(x)
                    ys.append(y)
        if not xs:
            return None
        return (min(xs), max(xs)), (min(ys), max(ys))

    def points(self, axes, box):
        out = []
        for a, b, _ in self._ends():
            out += [p for p in (axes._place(a[0], a[1], box), axes._place(b[0], b[1], box)) if p]
        return out

    def draw(self, out, axes, box):
        s = self.style
        bars, caps = [], []
        cap = s.get('capsize', 0.0) / 2.0
        for a, b, horizontal in self._ends():
            p = axes._place(a[0], a[1], box)
            q = axes._place(b[0], b[1], box)
            if p is None or q is None:
                continue
            bars.append((p[0], p[1], q[0], q[1]))
            if cap:
                for x, y in (p, q):
                    caps.append((x, y - cap, x, y + cap) if horizontal else (x - cap, y, x + cap, y))
        opacity = _opacity(s)
        _segments(out, bars, s['color'], s['linewidth'], opacity)
        _segments(out, caps, s['color'], s.get('capthick') or s['linewidth'], opacity)


class _ErrorbarContainer(tuple):
    """What errorbar() gives: (line, caps, bars), with the label."""

    def __new__(cls, line, bars, label, has_x, has_y):
        self = tuple.__new__(cls, (line, (), (bars,)))
        self.line = line
        self.bars = bars
        self._label = None if label is None else str(label)
        self.has_xerr = has_x
        self.has_yerr = has_y
        return self

    def get_label(self):
        return self._label or ''

    def set_label(self, label):
        self._label = None if label is None else str(label)

    def _look(self):
        look = dict(self.line.style, kind='errorbar')
        look['ecolor'] = self.bars.style['color']
        look['vertical_bar'] = self.has_yerr or not self.has_xerr
        return look


def _hist_edges(sets, bins, rng):
    bins = PLAIN(bins)
    if isinstance(bins, bool) or not isinstance(bins, (int, list, tuple)):
        raise TypeError('gossia.pyplot takes bins as a number or as the edges, not %r' % (bins,))
    if isinstance(bins, int):
        if bins < 1:
            raise ValueError('bins is at least 1')
        values = [v for d in sets for v in d]
        if rng is not None:
            lo, hi = [float(v) for v in rng]
        elif values:
            lo, hi = min(values), max(values)
        else:
            lo, hi = 0.0, 1.0
        if lo == hi:
            lo, hi = lo - 0.5, hi + 0.5
        return [lo + (hi - lo) * i / bins for i in _range(bins + 1)]
    edges = [float(v) for v in bins]
    if len(edges) < 2 or any(b <= a for a, b in zip(edges, edges[1:])):
        raise ValueError('the bin edges must rise, and there must be at least two')
    return edges


def _hist_counts(values, edges, weights):
    counts = [0.0] * (len(edges) - 1)
    if weights is not None and len(weights) != len(values):
        raise ValueError('there are %d weights for %d values' % (len(weights), len(values)))
    for i, v in enumerate(values):
        if v < edges[0] or v > edges[-1]:
            continue
        lo, hi = 0, len(edges) - 1
        while hi - lo > 1:
            mid = (lo + hi) // 2
            if v >= edges[mid]:
                lo = mid
            else:
                hi = mid
        counts[lo] += float(weights[i]) if weights is not None else 1.0
    return counts


class _Polygon(_Handle, _Artist):
    """Filled areas: fill_between, and the outlines of step histograms."""

    def __init__(self, polys, style, sticky_value=None, vertical=True):
        _Handle.__init__(self, style)
        self.polys = polys
        self.sticky_value = sticky_value
        self.vertical = vertical

    def entries(self):
        return [self]

    def bounds(self, axes):
        xs = [x for px, _ in self.polys for x in px]
        ys = [y for _, py in self.polys for y in py]
        return _points_bounds(xs, ys, axes)

    def sticky(self):
        if self.sticky_value is None:
            return [], []
        return ([], [self.sticky_value]) if self.vertical else ([self.sticky_value], [])

    def points(self, axes, box):
        out = []
        for px, py in self.polys:
            for run in _runs(px, py, axes, box):
                out += _sample(run)
        return out

    def draw(self, out, axes, box):
        s = self.style
        if s.get('visible') is False:
            return
        closed = s.get('closed', True)
        for px, py in self.polys:
            for run in _runs(px, py, axes, box):
                if len(run) < 2:
                    continue
                d = _path(run) + (' Z' if closed else '')
                face = s.get('facecolor') if closed else 'none'
                out.append('<path d="%s" fill="%s"%s%s stroke-linejoin="round"/>'
                           % (d, face or 'none', _stroke(s), _opacity(s)))


_PROP_KEYS = {'color': 'color', 'c': 'color', 'linewidth': 'linewidth', 'lw': 'linewidth',
              'linestyle': 'dash', 'ls': 'dash', 'facecolor': 'facecolor', 'fc': 'facecolor',
              'edgecolor': 'color', 'ec': 'color', 'marker': 'marker',
              'markerfacecolor': 'markerfacecolor', 'mfc': 'markerfacecolor',
              'markeredgecolor': 'markeredgecolor', 'mec': 'markeredgecolor',
              'markersize': 'markersize', 'ms': 'markersize', 'alpha': 'alpha'}


def _props(given, defaults):
    """A box plot part's look: matplotlib's boxprops, medianprops and the
    like, over the defaults."""
    style = dict(defaults)
    for key, value in dict(PLAIN(given) or {}).items():
        if key not in _PROP_KEYS:
            raise TypeError('gossia.pyplot does not support %s in the box plot properties' % key)
        name = _PROP_KEYS[key]
        if name in ('color', 'facecolor', 'markerfacecolor', 'markeredgecolor'):
            value = _colour(value)
        elif name == 'dash':
            value = _dash(value)
        elif name == 'marker':
            value = _marker_name(value)
        elif value is not None:
            value = float(value)
        style[name] = value
    return style


def _quantile(values, p):
    """numpy's default quantile: linear between the sorted values."""
    at = p * (len(values) - 1)
    lo = int(_math.floor(at))
    hi = min(lo + 1, len(values) - 1)
    return values[lo] + (values[hi] - values[lo]) * (at - lo)


def _box_stats(values, whis):
    if not values:
        return None
    q1, med, q3 = _quantile(values, 0.25), _quantile(values, 0.5), _quantile(values, 0.75)
    iqr = q3 - q1
    inside = [v for v in values if q1 - whis * iqr <= v <= q3 + whis * iqr]
    low = min(inside) if inside else q1
    high = max(inside) if inside else q3
    return {'q1': q1, 'med': med, 'q3': q3, 'whislo': min(low, q1), 'whishi': max(high, q3),
            'fliers': [v for v in values if v < low or v > high],
            'mean': sum(values) / len(values)}


class _Box:
    def __init__(self, position, width, stats, vertical, props, showmeans, meanline,
                 showfliers, showcaps, showbox):
        self.position, self.width, self.stats = position, width, stats
        self.vertical = vertical
        self.props = props
        self.show = {'means': showmeans, 'meanline': meanline, 'fliers': showfliers,
                     'caps': showcaps, 'box': showbox}
        self.handles = {k: _Handle(v) for k, v in props.items()}

    def _at(self, axes, box, p, v):
        return axes._place(p, v, box) if self.vertical else axes._place(v, p, box)

    def extent(self):
        s = self.stats
        if s is None:
            return None
        values = [s['whislo'], s['whishi']] + (s['fliers'] if self.show['fliers'] else [])
        return ((self.position - self.width / 2.0, self.position + self.width / 2.0),
                (min(values), max(values)))

    def _line(self, out, axes, box, part, pairs, dash=None):
        st = self.props[part]
        segs = []
        for (a1, b1), (a2, b2) in pairs:
            u, w = self._at(axes, box, a1, b1), self._at(axes, box, a2, b2)
            if u and w:
                segs.append((u[0], u[1], w[0], w[1]))
        _segments(out, segs, st.get('color') or 'currentColor', st.get('linewidth', 1.0),
                  _opacity(st), st.get('dash') or dash)

    def draw(self, out, axes, box):
        s = self.stats
        if s is None:
            return
        p, half = self.position, self.width / 2.0
        props = self.props
        self._line(out, axes, box, 'whisker',
                   [((p, s['q1']), (p, s['whislo'])), ((p, s['q3']), (p, s['whishi']))])
        if self.show['caps']:
            self._line(out, axes, box, 'cap',
                       [((p - half / 2, s['whislo']), (p + half / 2, s['whislo'])),
                        ((p - half / 2, s['whishi']), (p + half / 2, s['whishi']))])
        if self.show['box']:
            a = self._at(axes, box, p - half, s['q1'])
            b = self._at(axes, box, p + half, s['q3'])
            if a and b:
                st = props['box']
                width = st.get('linewidth', 1.0)
                out.append('<rect x="%s" y="%s" width="%s" height="%s" fill="%s" stroke="%s"'
                           ' stroke-width="%s"%s%s/>'
                           % (_fmt(min(a[0], b[0])), _fmt(min(a[1], b[1])), _fmt(abs(b[0] - a[0])),
                              _fmt(abs(b[1] - a[1])), st.get('facecolor') or 'none',
                              st.get('color') or 'currentColor', _fmt(width),
                              _dash_attr(st.get('dash'), width), _opacity(st)))
        self._line(out, axes, box, 'median', [((p - half, s['med']), (p + half, s['med']))])
        if self.show['fliers']:
            st = props['flier']
            if st.get('marker'):
                for v in s['fliers']:
                    q = self._at(axes, box, p, v)
                    if q:
                        out.append(_marker(st['marker'], q[0], q[1], st.get('markersize', 6.0),
                                           st.get('markerfacecolor') or 'none',
                                           st.get('markeredgecolor') or st.get('color') or 'currentColor',
                                           _opacity(st)))
        if self.show['means']:
            st = props['mean']
            if self.show['meanline']:
                self._line(out, axes, box, 'mean', [((p - half, s['mean']), (p + half, s['mean']))],
                           _LINESTYLES['--'])
            else:
                q = self._at(axes, box, p, s['mean'])
                if q:
                    out.append(_marker(st.get('marker') or '^', q[0], q[1], st.get('markersize', 6.0),
                                       st.get('markerfacecolor'), st.get('markeredgecolor'), _opacity(st)))


class _BoxPlot(_Artist):
    def __init__(self, boxes, vertical):
        self.boxes = boxes
        self.vertical = vertical

    def bounds(self, axes):
        found = [b.extent() for b in self.boxes if b.extent() is not None]
        if not found:
            return None
        pos = (min(f[0][0] for f in found), max(f[0][1] for f in found))
        val = (min(f[1][0] for f in found), max(f[1][1] for f in found))
        return (pos, val) if self.vertical else (val, pos)

    def points(self, axes, box):
        out = []
        for b in self.boxes:
            e = b.extent()
            if e is None:
                continue
            for i in range(3):
                for j in range(5):
                    p = e[0][0] + (e[0][1] - e[0][0]) * i / 2.0
                    v = e[1][0] + (e[1][1] - e[1][0]) * j / 4.0
                    q = b._at(axes, box, p, v)
                    if q:
                        out.append(q)
        return out

    def draw(self, out, axes, box):
        for b in self.boxes:
            b.draw(out, axes, box)


class _Wedge(_Handle):
    def __init__(self, style, cx, cy, r, theta1, theta2, width):
        _Handle.__init__(self, style)
        self.cx, self.cy, self.r = cx, cy, r
        self.theta1, self.theta2 = theta1, theta2
        self.width = width

    def path(self, axes, box):
        c = axes._place(self.cx, self.cy, box)
        if c is None:
            return None
        left, top, right, bottom = box
        xlim = axes._view[0]
        scale = (right - left) / abs(xlim[1] - xlim[0])
        R = self.r * scale
        inner = (self.r - self.width) * scale if self.width else 0.0

        def at(radius, theta):
            t = _math.radians(theta)
            return '%s %s' % (_fmt(c[0] + radius * _math.cos(t)), _fmt(c[1] - radius * _math.sin(t)))

        span = self.theta2 - self.theta1
        if span >= 359.999:
            ring = ('M%s A%s %s 0 1 0 %s A%s %s 0 1 0 %s Z'
                    % (at(R, 0), _fmt(R), _fmt(R), at(R, 180), _fmt(R), _fmt(R), at(R, 0)))
            if inner > 0:
                ring += (' M%s A%s %s 0 1 1 %s A%s %s 0 1 1 %s Z'
                         % (at(inner, 0), _fmt(inner), _fmt(inner), at(inner, 180),
                            _fmt(inner), _fmt(inner), at(inner, 0)))
            return ring
        large = 1 if span > 180 else 0
        if inner > 0:
            return ('M%s A%s %s 0 %d 0 %s L%s A%s %s 0 %d 1 %s Z'
                    % (at(R, self.theta1), _fmt(R), _fmt(R), large, at(R, self.theta2),
                       at(inner, self.theta2), _fmt(inner), _fmt(inner), large, at(inner, self.theta1)))
        return ('M%s %s L%s A%s %s 0 %d 0 %s Z'
                % (_fmt(c[0]), _fmt(c[1]), at(R, self.theta1), _fmt(R), _fmt(R), large,
                   at(R, self.theta2)))


class _Pie(_Artist):
    def __init__(self, wedges, texts):
        self.wedges = wedges
        self.texts = texts

    def entries(self):
        return list(self.wedges)

    def bounds(self, axes):
        if not self.wedges:
            return None
        return ((min(w.cx - w.r for w in self.wedges), max(w.cx + w.r for w in self.wedges)),
                (min(w.cy - w.r for w in self.wedges), max(w.cy + w.r for w in self.wedges)))

    def points(self, axes, box):
        out = []
        for w in self.wedges:
            for k in range(5):
                t = _math.radians(w.theta1 + (w.theta2 - w.theta1) * k / 4.0)
                for f in (0.35, 0.7, 1.0):
                    p = axes._place(w.cx + f * w.r * _math.cos(t), w.cy + f * w.r * _math.sin(t), box)
                    if p:
                        out.append(p)
        return out

    def draw(self, out, axes, box):
        for w in self.wedges:
            if w.style.get('visible') is False:
                continue
            d = w.path(axes, box)
            if d:
                out.append('<path d="%s" fill="%s" fill-rule="evenodd"%s%s stroke-linejoin="round"/>'
                           % (d, w.style.get('facecolor') or 'none', _stroke(w.style, 0.0),
                              _opacity(w.style)))

    def draw_over(self, out, axes, box):
        for t in self.texts:
            t.draw_over(out, axes, box)


def _text_svg(out, x, y, style):
    """Text as text() draws it, one <text> per line."""
    if style.get('visible') is False:
        return
    lines = str(style.get('text', '')).split('\n')
    size = style.get('fontsize', 10.0)
    step = size * _LINE
    va = style.get('va', 'baseline')
    n = len(lines)
    if va == 'top':
        first = y + size * _ASCENT
    elif va in ('center', 'center_baseline'):
        first = y - ((n - 1) * step + size * _ASCENT) / 2.0 + size * _ASCENT
    elif va == 'bottom':
        first = y - (n - 1) * step - size * 0.22
    else:
        first = y - (n - 1) * step
    anchor = {'left': 'start', 'center': 'middle', 'right': 'end'}[style.get('ha', 'left')]
    attrs = ''
    weight = style.get('fontweight')
    if weight not in (None, 'normal'):
        attrs += ' font-weight="%s"' % ('bold' if weight in ('bold', 'heavy', 'semibold', 'demibold')
                                         else _esc(weight))
    if style.get('fontstyle') in ('italic', 'oblique'):
        attrs += ' font-style="italic"'
    if style.get('alpha') is not None:
        attrs += ' fill-opacity="%s"' % _fmt(style['alpha'])
    if style.get('rotation'):
        attrs += ' transform="rotate(%s %s %s)"' % (_fmt(-style['rotation']), _fmt(x), _fmt(y))
    colour = style.get('color') or 'currentColor'
    for i, line in enumerate(lines):
        out.append('<text x="%s" y="%s" font-size="%s" text-anchor="%s" fill="%s"%s>%s</text>'
                   % (_fmt(x), _fmt(first + i * step), _fmt(size), anchor, colour, attrs, _esc(line)))


def _text_box(x, y, style):
    """The rectangle text takes up, unrotated: (left, top, right, bottom)."""
    lines = str(style.get('text', '')).split('\n')
    size = style.get('fontsize', 10.0)
    w = max(_width(l, size, style.get('fontweight') == 'bold') for l in lines)
    h = (len(lines) - 1) * size * _LINE + size * (_ASCENT + 0.22)
    ha, va = style.get('ha', 'left'), style.get('va', 'baseline')
    left = {'left': x, 'center': x - w / 2.0, 'right': x - w}[ha]
    if va == 'top':
        top = y
    elif va in ('center', 'center_baseline'):
        top = y - h / 2.0
    elif va == 'bottom':
        top = y - h
    else:
        top = y - (len(lines) - 1) * size * _LINE - size * _ASCENT
    return left, top, left + w, top + h


class _Text(_Handle, _Artist):
    _kind = 'text'

    def __init__(self, x, y, style, transform='data'):
        _Handle.__init__(self, style)
        self.x, self.y = x, y
        self.transform = transform

    def get_position(self):
        return (self.x, self.y)

    def set_position(self, xy):
        self.x, self.y = xy

    def set_color(self, color):
        self.style['color'] = _colour(color)

    def draw_over(self, out, axes, box):
        p = axes._at(self.x, self.y, self.transform, box)
        if p is not None:
            _text_svg(out, p[0], p[1], self.style)


class _SpanLine(_Handle, _Artist):
    """axhline and axvline: across the plot box, at a value."""
    _kind = 'line'

    def __init__(self, value, start, end, horizontal, style):
        _Handle.__init__(self, style)
        self.value, self.start, self.end = value, start, end
        self.horizontal = horizontal

    def entries(self):
        return [self]

    def bounds(self, axes):
        if self.value is None:
            return None
        return (None, (self.value, self.value)) if self.horizontal else ((self.value, self.value), None)

    def _ends(self, axes, box):
        left, top, right, bottom = box
        if self.horizontal:
            y = axes._pos(axes.yaxis, self.value, box)
            if y is None:
                return None
            return [(left + (right - left) * self.start, y), (left + (right - left) * self.end, y)]
        x = axes._pos(axes.xaxis, self.value, box)
        if x is None:
            return None
        return [(x, bottom - (bottom - top) * self.start), (x, bottom - (bottom - top) * self.end)]

    def points(self, axes, box):
        ends = self._ends(axes, box)
        return _sample(ends) if ends else []

    def draw(self, out, axes, box):
        s = self.style
        ends = self._ends(axes, box)
        if ends is None or s.get('visible') is False or s.get('dash') == 'none':
            return
        width = s.get('linewidth', 1.5)
        out.append('<path d="%s" fill="none" stroke="%s" stroke-width="%s"%s%s/>'
                   % (_path(ends), s['color'], _fmt(width), _dash_attr(s.get('dash'), width),
                      _opacity(s)))


class _Span(_Handle, _Artist):
    """axhspan and axvspan: a band across the plot box."""

    def __init__(self, lo, hi, start, end, horizontal, style):
        _Handle.__init__(self, style)
        self.lo, self.hi, self.start, self.end = lo, hi, start, end
        self.horizontal = horizontal

    def entries(self):
        return [self]

    def bounds(self, axes):
        r = (min(self.lo, self.hi), max(self.lo, self.hi))
        return (None, r) if self.horizontal else (r, None)

    def _rect(self, axes, box):
        left, top, right, bottom = box
        axis = axes.yaxis if self.horizontal else axes.xaxis
        a, b = axes._pos(axis, self.lo, box, True), axes._pos(axis, self.hi, box, True)
        if a is None or b is None:
            return None
        if self.horizontal:
            x0 = left + (right - left) * self.start
            x1 = left + (right - left) * self.end
            return x0, min(a, b), x1 - x0, abs(b - a)
        y0 = bottom - (bottom - top) * self.end
        y1 = bottom - (bottom - top) * self.start
        return min(a, b), y0, abs(b - a), y1 - y0

    def points(self, axes, box):
        r = self._rect(axes, box)
        if r is None:
            return []
        x, y, w, h = r
        return [(x + w * i / 2.0, y + h * j / 2.0) for i in range(3) for j in range(3)]

    def draw(self, out, axes, box):
        r = self._rect(axes, box)
        if r is None or self.style.get('visible') is False:
            return
        x, y, w, h = r
        out.append('<rect x="%s" y="%s" width="%s" height="%s" fill="%s"%s%s/>'
                   % (_fmt(x), _fmt(y), _fmt(w), _fmt(h), self.style['facecolor'],
                      _stroke(self.style, 0.0), _opacity(self.style)))


class _Annotation(_Handle, _Artist):
    _kind = 'text'

    def __init__(self, point, point_coords, place, place_coords, style, arrow):
        _Handle.__init__(self, style)
        self.point, self.point_coords = point, point_coords
        self.place, self.place_coords = place, place_coords
        self.arrow = arrow

    def draw_over(self, out, axes, box):
        p = axes._at(self.point[0], self.point[1], self.point_coords, box)
        if p is None:
            return
        if self.place_coords == 'offset points':
            t = (p[0] + self.place[0], p[1] - self.place[1])
        else:
            t = axes._at(self.place[0], self.place[1], self.place_coords, box)
            if t is None:
                return
        if self.arrow is not None:
            _arrow(out, t, p, _text_box(t[0], t[1], self.style), self.arrow)
        _text_svg(out, t[0], t[1], self.style)


def _arrow(out, start, end, text_box, arrow):
    """An annotation's arrow, from the edge of its text to the point."""
    x0, y0 = start
    x1, y1 = end
    dx, dy = x1 - x0, y1 - y0
    length = _math.hypot(dx, dy)
    if length < 1e-6:
        return
    # Leave the text's box, and 2 points more.
    left, top, right, bottom = text_box
    t_out = 0.0
    if left <= x0 <= right and top <= y0 <= bottom:
        exits = []
        if dx > 0:
            exits.append((right - x0) / dx)
        elif dx < 0:
            exits.append((left - x0) / dx)
        if dy > 0:
            exits.append((bottom - y0) / dy)
        elif dy < 0:
            exits.append((top - y0) / dy)
        t_out = min(exits) if exits else 0.0
    t_out = min(t_out, 1.0) + 2.0 / length
    shrink = arrow['shrink']
    ta = max(t_out, shrink)
    tb = 1.0 - shrink
    if tb <= ta:
        return
    ux, uy = dx / length, dy / length
    sx, sy = x0 + dx * ta, y0 + dy * ta
    ex, ey = x0 + dx * tb, y0 + dy * tb
    colour = arrow['color'] or 'currentColor'
    opacity = ' opacity="%s"' % _fmt(float(arrow['alpha'])) if arrow.get('alpha') is not None else ''
    style = arrow['style']
    width = arrow['linewidth']
    head_len, head_half = (arrow['headlength'], arrow['headwidth'] / 2.0) if arrow.get('old') \
        else (4.0 + width, 2.0 + width / 2.0)
    heads = []
    if style.endswith('>'):
        heads.append(((ex, ey), (ux, uy), style.endswith('|>')))
    if style.startswith('<'):
        heads.append(((sx, sy), (-ux, -uy), style.startswith('<|')))
    # Stop the shaft short of a filled head.
    ax0, ay0, ax1, ay1 = sx, sy, ex, ey
    for (hx, hy), (vx, vy), filled in heads:
        if filled:
            if (hx, hy) == (ex, ey):
                ax1, ay1 = ex - vx * head_len * 0.9, ey - vy * head_len * 0.9
            else:
                ax0, ay0 = sx - vx * head_len * 0.9, sy - vy * head_len * 0.9
    out.append('<path d="M%s %s L%s %s" fill="none" stroke="%s" stroke-width="%s"%s/>'
               % (_fmt(ax0), _fmt(ay0), _fmt(ax1), _fmt(ay1), colour, _fmt(width), opacity))
    for (hx, hy), (vx, vy), filled in heads:
        bx, by = hx - vx * head_len, hy - vy * head_len
        px, py = -vy * head_half, vx * head_half
        if filled:
            out.append('<polygon points="%s,%s %s,%s %s,%s" fill="%s"%s/>'
                       % (_fmt(hx), _fmt(hy), _fmt(bx + px), _fmt(by + py), _fmt(bx - px),
                          _fmt(by - py), colour, opacity))
        else:
            out.append('<path d="M%s %s L%s %s L%s %s" fill="none" stroke="%s" stroke-width="%s"'
                       ' stroke-linejoin="miter"%s/>'
                       % (_fmt(bx + px), _fmt(by + py), _fmt(hx), _fmt(hy), _fmt(bx - px),
                          _fmt(by - py), colour, _fmt(width), opacity))


class _Legend:
    """What legend() gives."""

    def __init__(self, entries, opts):
        self.entries = entries
        self.opts = opts

    def __getattr__(self, name):
        raise _refuse('Legend.%s' % name)

    def set_title(self, title, prop=None):
        self.opts['title'] = None if title is None else str(title)

    def get_texts(self):
        return [label for _, label in self.entries]

    def set_visible(self, visible):
        self.opts['visible'] = bool(visible)

    def remove(self):
        self.entries = []


def _look(handle):
    """How a legend shows a handle."""
    look = getattr(type(handle), '_look', None)
    if look is not None:
        return look(handle)
    if isinstance(handle, _Handle):
        return dict(handle.style, kind=handle._kind)
    raise TypeError('a legend handle is something drawn, such as a line plot() gave')


def _legend_handle(out, look, x, y, length, size, scale):
    """The sample of a legend entry, `length` long from x, centred on y."""
    if look is None:
        return
    kind = look['kind']
    opacity = _opacity(look)
    if kind == 'patch':
        h = size * 0.7
        face = look.get('facecolor') or look.get('color') or 'none'
        out.append('<rect x="%s" y="%s" width="%s" height="%s" fill="%s"%s%s/>'
                   % (_fmt(x), _fmt(y - h / 2), _fmt(length), _fmt(h), face,
                      _stroke(look, 1.0) if look.get('edgecolor') else '', opacity))
        return
    if kind == 'marker':
        out.append(_marker(look['marker'] or 'o', x + length / 2, y, look['markersize'] * scale,
                           look['markerfacecolor'], look['markeredgecolor'], opacity))
        return
    colour = look.get('color') or 'currentColor'
    width = look.get('linewidth', 1.5)
    if look.get('dash') != 'none':
        out.append('<path d="M%s %s H%s" fill="none" stroke="%s" stroke-width="%s"%s%s/>'
                   % (_fmt(x), _fmt(y), _fmt(x + length), colour, _fmt(width),
                      _dash_attr(look.get('dash'), width), opacity))
    if kind == 'errorbar':
        half = size * 0.5
        c = x + length / 2
        seg = ((c, y - half, c, y + half) if look.get('vertical_bar')
               else (x + length * 0.2, y, x + length * 0.8, y))
        _segments(out, [seg], look.get('ecolor') or colour, width, opacity)
    if look.get('marker'):
        out.append(_marker(look['marker'], x + length / 2, y, look.get('markersize', 6.0) * scale,
                           look.get('markerfacecolor') or colour,
                           look.get('markeredgecolor') or colour, opacity))


def _legend_size(legend):
    """The legend's width and height, and the parts of its layout."""
    o = legend.opts
    size = o['fontsize']
    entries = legend.entries
    ncol = max(1, min(o['ncol'], len(entries)))
    nrows = (len(entries) + ncol - 1) // ncol
    cols = [entries[i * nrows:(i + 1) * nrows] for i in range(ncol)]
    cols = [c for c in cols if c]
    pad, handle, gap, spacing, columns = 0.4 * size, 2.0 * size, 0.8 * size, 0.5 * size, 2.0 * size
    widths = [handle + gap + max(_width(label, size) for _, label in c) for c in cols]
    title = o.get('title')
    tsize = o['title_fontsize']
    title_h = tsize * _LINE if title else 0.0
    w = 2 * pad + sum(widths) + columns * (len(widths) - 1)
    if title:
        w = max(w, 2 * pad + _width(title, tsize))
    h = 2 * pad + title_h + nrows * size + (nrows - 1) * spacing
    return w, h, cols, widths, title_h


def _draw_legend(out, ax, box):
    legend = ax.legend_
    if legend is None or not legend.entries or legend.opts.get('visible') is False:
        return
    o = legend.opts
    size = o['fontsize']
    pad, handle, gap, spacing, columns, margin = (0.4 * size, 2.0 * size, 0.8 * size,
                                                  0.5 * size, 2.0 * size, 0.5 * size)
    w, h, cols, widths, title_h = _legend_size(legend)
    left, top, right, bottom = box
    xs = {'left': left + margin, 'center': (left + right - w) / 2.0, 'right': right - margin - w}
    ys = {'upper': top + margin, 'center': (top + bottom - h) / 2.0, 'lower': bottom - margin - h}

    def place(loc):
        if loc == 'center':
            return xs['center'], ys['center']
        if loc == 'right':
            loc = 'center right'
        v, hz = loc.split(' ')
        return xs[hz], ys[v]

    loc = o['loc']
    if loc == 'best':
        # The first place, in matplotlib's order, that covers the fewest
        # points of what is drawn.
        points = []
        host = ax.host or ax
        for owner in (host, host.twin):
            if owner is not None:
                for artist in owner.artists:
                    points.extend(artist.points(owner, box))
        best = None
        for candidate in _LEGEND_CODES[1:]:
            lx, ly = place(candidate)
            count = sum(1 for px, py in points if lx <= px <= lx + w and ly <= py <= ly + h)
            if best is None or count < best[0]:
                best = (count, lx, ly)
        lx, ly = best[1], best[2]
    else:
        lx, ly = place(loc)
    if o['frameon']:
        face = o['facecolor']
        fill = ('fill="%s"' % face) if face else 'fill="%s" class="surface"' % _SURFACE
        edge = o['edgecolor']
        stroke = ('stroke="%s"' % edge) if edge else 'stroke="currentColor" stroke-opacity="0.2"'
        out.append('<rect x="%s" y="%s" width="%s" height="%s" rx="2" %s fill-opacity="%s" %s'
                   ' stroke-width="0.8"/>' % (_fmt(lx), _fmt(ly), _fmt(w), _fmt(h), fill,
                                              _fmt(o['framealpha']), stroke))
    title = o.get('title')
    if title:
        tsize = o['title_fontsize']
        _text(out, lx + w / 2, ly + pad + tsize * _ASCENT, title, tsize)
    x = lx + pad
    for c, column in enumerate(cols):
        for r, (handle_, label) in enumerate(column):
            cy = ly + pad + title_h + r * (size + spacing) + size / 2.0
            look = _look(handle_)
            _legend_handle(out, look, x, cy, handle, size, o['markerscale'])
            colour = 'currentColor'
            labelcolor = o.get('labelcolor')
            if labelcolor in ('linecolor', 'markerfacecolor', 'mfc'):
                if look:
                    colour = (look.get('color') or look.get('facecolor')
                              or look.get('markerfacecolor') or colour)
            elif labelcolor is not None:
                colour = _colour(labelcolor)
            out.append('<text x="%s" y="%s" font-size="%s" text-anchor="start" fill="%s">%s</text>'
                       % (_fmt(x + handle + gap), _fmt(cy + size * _ASCENT / 2), _fmt(size),
                          colour, _esc(label)))
        x += widths[c] + columns


# ------------------------------------------------------------- the world

# The world outline (DESIGN.md, "The world outline"): Natural Earth's land and
# country borders at 1:110m, given by Gossia as the text `_WORLD_TEXT` and
# read the first time a map is drawn.
_world_cache = []


def _world_shapes():
    """The land rings and the border lines, each as (xs, ys, (xmin, xmax,
    ymin, ymax)) in degrees of longitude and latitude."""
    if not _world_cache:
        land, borders = [], []
        for row in _WORLD_TEXT.split('\n'):
            if not row or row[0] == '#':
                continue
            parts = row.split(' ')
            numbers = [int(n) for n in parts[1:]]
            xs, ys = [], []
            x = y = 0
            for i in _range(0, len(numbers) - 1, 2):
                x += numbers[i]
                y += numbers[i + 1]
                xs.append(x / 100.0)
                ys.append(y / 100.0)
            shape = (xs, ys, (min(xs), max(xs), min(ys), max(ys)))
            (land if parts[0] == 'L' else borders).append(shape)
        _world_cache.append((land, borders))
    return _world_cache[0]


_MERCATOR_LIMIT = 85.0


def _mercator(latitude):
    """Where a latitude goes on Mercator's map, in degrees at the equator, so
    that an equal aspect keeps the shapes; cut at 85 degrees."""
    v = max(-_MERCATOR_LIMIT, min(_MERCATOR_LIMIT, float(latitude)))
    return _math.degrees(_math.log(_math.tan(_math.pi / 4.0 + _math.radians(v) / 2.0)))


class _World(_Artist):
    """The land and the borders, behind what else the Axes has."""

    def __init__(self, style):
        self.style = style

    def bounds(self, axes):
        # The whole world, unless something else on the Axes says where to
        # look: then that, with room around it (a fifth of its spread, and
        # at least ten degrees, each way), within the world.
        xs, ys = [], []
        for artist in axes.artists:
            if artist is self or isinstance(artist, _World):
                continue
            b = artist.bounds(axes)
            if b is None:
                continue
            if b[0] is not None:
                xs.extend(b[0])
            if b[1] is not None:
                ys.extend(b[1])

        def around(values, lo, hi):
            if not values:
                return lo, hi
            a, b = min(values), max(values)
            pad = max((b - a) * 0.2, 10.0)
            return max(a - pad, lo), min(b + pad, hi)

        return around(xs, -180.0, 180.0), around(ys, -90.0, 90.0)

    def sticky(self):
        return [-180.0, 180.0], [-90.0, 90.0]

    def draw(self, out, axes, box):
        s = self.style
        (xlo, xhi), (ylo, yhi) = [(min(v), max(v)) for v in axes._view]
        land, borders = _world_shapes()

        def seen(bbox):
            return bbox[1] >= xlo and bbox[0] <= xhi and bbox[3] >= ylo and bbox[2] <= yhi

        def paths(shapes, closed):
            d = []
            for xs, ys, bbox in shapes:
                if not seen(bbox):
                    continue
                for run in _runs(xs, ys, axes, box):
                    if len(run) >= 2:
                        d.append(_path(run) + (' Z' if closed else ''))
            return ' '.join(d)

        d = paths(land, True)
        if d:
            out.append('<path d="%s" fill="%s"%s stroke="%s" stroke-width="%s"%s'
                       ' stroke-linejoin="round"/>'
                       % (d, s['land'], s['land_opacity'], s['edge'], _fmt(s['linewidth']),
                          s['edge_opacity']))
        if s['borders']:
            d = paths(borders, False)
            if d:
                out.append('<path d="%s" fill="none" stroke="%s" stroke-width="%s"%s'
                           ' stroke-linejoin="round"/>'
                           % (d, s['edge'], _fmt(s['linewidth'] * 0.8), s['border_opacity']))


class Axes:
    """A plot area of a figure, with its x and y axes."""

    def __init__(self, figure, row, col, rows, cols):
        self.figure = figure
        self.cell = (row, col, rows, cols)
        self.xaxis = _Axis('x', self)
        self.yaxis = _Axis('y', self)
        self.twin = None          # the Axes twinx() made of this one
        self.host = None          # for a twin, the Axes it was made from
        self._reset()

    def _reset(self):
        self.artists = []
        self.containers = []
        self.titles = {}
        self.grid_on = {'x': False, 'y': False}
        self.grid_style = {}
        self.cycle = 0
        self.legend_ = None
        self.frame_on = True
        self.axis_on = True
        self.aspect = None
        self.hide = {'x': False, 'y': False}

    def __getattr__(self, name):
        raise _refuse('Axes.%s' % name)

    # Where text() and annotate() take their positions: the data's
    # coordinates (the default), or fractions of the plot box.
    transData = 'data'
    transAxes = 'axes fraction'

    # -- what is drawn

    def _next_colour(self):
        colour = _CYCLE[self.cycle % len(_CYCLE)]
        self.cycle += 1
        return colour

    def _add(self, artist):
        self.artists.append(artist)
        return artist

    _LINE_ARGS = frozenset(['color', 'c', 'linestyle', 'ls', 'linewidth', 'lw', 'label',
                            'marker', 'markersize', 'ms', 'markerfacecolor', 'mfc',
                            'markeredgecolor', 'mec', 'markeredgewidth', 'mew',
                            'alpha', 'drawstyle', 'ds'])

    def _line_style(self, kwargs, color=None, marker=None, line=None, default_colour=None):
        """The style of a line from a format string's parts and the keyword
        arguments, which win."""
        style = {}
        colour = kwargs.get('color', kwargs.get('c', color))
        if colour is not None:
            style['color'] = _colour(colour)
        else:
            style['color'] = default_colour or self._next_colour()
        style['dash'] = _dash(kwargs.get('linestyle', kwargs.get('ls', line)))
        style['linewidth'] = float(kwargs.get('linewidth', kwargs.get('lw', 1.5)))
        style['marker'] = _marker_name(kwargs.get('marker', marker))
        style['markersize'] = float(kwargs.get('markersize', kwargs.get('ms', 6.0)))
        for key, short in (('markerfacecolor', 'mfc'), ('markeredgecolor', 'mec')):
            value = kwargs.get(key, kwargs.get(short))
            style[key] = _colour(value) if value is not None else None
        style['markeredgewidth'] = float(kwargs.get('markeredgewidth', kwargs.get('mew', 1.0)))
        if kwargs.get('alpha') is not None:
            style['alpha'] = float(kwargs['alpha'])
        if kwargs.get('label') is not None:
            style['label'] = str(kwargs['label'])
        drawstyle = kwargs.get('drawstyle', kwargs.get('ds'))
        if drawstyle not in (None, 'default'):
            steps = {'steps': 'pre', 'steps-pre': 'pre', 'steps-post': 'post',
                     'steps-mid': 'mid'}.get(drawstyle)
            if steps is None:
                raise ValueError("a drawstyle is 'default', 'steps-pre', 'steps-post' or 'steps-mid'")
            style['steps'] = steps
        return style

    def plot(self, *args, **kwargs):
        _check(kwargs, self._LINE_ARGS | {'scalex', 'scaley', 'data'}, 'plot')
        if kwargs.pop('data', None) is not None:
            raise TypeError('gossia.pyplot does not support the argument data of plot')
        kwargs.pop('scalex', None)
        kwargs.pop('scaley', None)
        groups = []
        args = list(args)
        while args:
            if len(args) >= 2 and not isinstance(args[1], str):
                x, y = args[0], args[1]
                args = args[2:]
            else:
                x, y = None, args[0]
                args = args[1:]
            fmt = None
            if args and isinstance(args[0], str):
                fmt = args.pop(0)
            groups.append((x, y, fmt))
        if not groups:
            raise TypeError('plot needs the values to draw')
        lines = []
        for x, y, fmt in groups:
            ys = self.yaxis.convert(y, 'the y values')
            if x is None:
                xs = [float(i) for i in range(len(ys))]
                self.xaxis._kind('number') if self.xaxis.kind is None else None
            else:
                xs = self.xaxis.convert(x, 'the x values')
            if len(xs) != len(ys):
                raise ValueError('x and y must have the same length, not %d and %d'
                                 % (len(xs), len(ys)))
            color = marker = line = None
            if fmt is not None:
                color, marker, line = _parse_format(fmt)
            lines.append(self._add(_Line(xs, ys, self._line_style(kwargs, color, marker, line))))
        return lines

    def step(self, x, y, *args, where='pre', **kwargs):
        if where not in ('pre', 'post', 'mid'):
            raise ValueError("where is 'pre', 'post' or 'mid'")
        kwargs['drawstyle'] = 'steps-' + where
        return self.plot(x, y, *args, **kwargs)

    def _xy(self, x, y, where):
        xs = self.xaxis.convert(x, 'the x values')
        ys = self.yaxis.convert(y, 'the y values')
        if len(xs) != len(ys):
            raise ValueError('%s: x and y must have the same length, not %d and %d'
                             % (where, len(xs), len(ys)))
        return xs, ys

    def scatter(self, x, y, s=None, c=None, marker=None, alpha=None, linewidths=None,
                edgecolors=None, *, label=None, color=None, **kwargs):
        _check(kwargs, {'facecolors', 'edgecolor', 'linewidth', 'lw', 'plotnonfinite'}, 'scatter')
        xs, ys = self._xy(x, y, 'scatter')
        n = len(xs)
        sizes = _per_point(s, n, 'the sizes s', float, 36.0)
        if c is None:
            c = color if color is not None else kwargs.get('facecolors')
        colours = _point_colours(c, n) if c is not None else [self._next_colour()] * n
        edges = edgecolors if edgecolors is not None else kwargs.get('edgecolor', 'face')
        if edges == 'face':
            edges = None
        elif edges is not None:
            edges = _point_colours(edges, n)
        width = linewidths if linewidths is not None else kwargs.get('linewidth', kwargs.get('lw', 1.0))
        style = {'marker': _marker_name(marker if marker is not None else 'o'),
                 'alpha': None if alpha is None else float(alpha),
                 'label': None if label is None else str(label),
                 'linewidth': float(width)}
        return self._add(_Scatter(xs, ys, sizes, colours, edges, style))

    def bar(self, x, height, width=0.8, bottom=None, *, align='center', **kwargs):
        return self._bars(x, height, width, bottom, align, kwargs, True, 'bar')

    def barh(self, y, width, height=0.8, left=None, *, align='center', **kwargs):
        return self._bars(y, width, height, left, align, kwargs, False, 'barh')

    def _bars(self, pos, length, thickness, base, align, kwargs, vertical, where):
        _check(kwargs, {'color', 'facecolor', 'fc', 'edgecolor', 'ec', 'linewidth', 'lw',
                        'tick_label', 'label', 'alpha', 'xerr', 'yerr', 'ecolor', 'capsize',
                        'log', 'linestyle', 'ls'}, where)
        if align not in ('center', 'edge'):
            raise ValueError("align is 'center' or 'edge'")
        paxis, vaxis = (self.xaxis, self.yaxis) if vertical else (self.yaxis, self.xaxis)
        if isinstance(PLAIN(pos), (str, int, float)) or isinstance(pos, _dt.date):
            pos = [pos]
        positions = paxis.convert(pos, 'the positions of the bars')
        n = len(positions)
        lengths = _per_point(length, n, 'the bar lengths', float, None)
        thicknesses = _per_point(thickness, n, 'the bar widths', float, None)
        if base is None:
            bases = [0.0] * n
            if vaxis.kind is None:
                vaxis._kind('number')
        else:
            if not isinstance(PLAIN(base), (list, tuple)):
                base = [base] * n
            bases = vaxis.convert(base, 'the bases of the bars')
            if len(bases) != n:
                raise ValueError('%s: there are %d bases for %d bars' % (where, len(bases), n))
        colour = kwargs.get('color', kwargs.get('facecolor', kwargs.get('fc')))
        faces = _point_colours(colour, n) if colour is not None else [self._next_colour()] * n
        edge = kwargs.get('edgecolor', kwargs.get('ec'))
        edges = _point_colours(edge, n) if edge is not None else [None] * n
        linewidth = float(kwargs.get('linewidth', kwargs.get('lw', 1.0 if edge is not None else 0.0)))
        alpha = kwargs.get('alpha')
        patches = []
        for i in range(n):
            p, t = positions[i], thicknesses[i]
            if p is None or lengths[i] is None:
                continue
            start = p - t / 2.0 if align == 'center' else p
            patches.append(_Bar({'facecolor': faces[i], 'edgecolor': edges[i],
                                 'linewidth': linewidth,
                                 'dash': _dash(kwargs.get('linestyle', kwargs.get('ls'))),
                                 'alpha': None if alpha is None else float(alpha)},
                                start, t, bases[i], lengths[i], vertical))
        bars = self._add(_Bars(patches, vertical))
        container = _BarContainer(patches, kwargs.get('label'))
        self.containers.append(container)
        if kwargs.get('tick_label') is not None:
            labels = kwargs['tick_label']
            labels = [labels] * n if isinstance(labels, str) else [str(l) for l in _list(labels, 'tick_label')]
            paxis.ticks = [p for p in positions if p is not None]
            paxis.tick_labels = labels
        if kwargs.get('log'):
            vaxis.scale = 'log'
        xerr, yerr = kwargs.get('xerr'), kwargs.get('yerr')
        if xerr is not None or yerr is not None:
            centres = [b.start + b.thickness / 2.0 for b in patches]
            ends = [b.base + b.length for b in patches]
            xs, ys = (centres, ends) if vertical else (ends, centres)
            ecolor = kwargs.get('ecolor')
            self._add(_ErrorBars(xs, ys, _errors(xerr, len(xs), 'xerr'), _errors(yerr, len(xs), 'yerr'),
                                 {'color': _colour(ecolor) if ecolor is not None else 'currentColor',
                                  'linewidth': 1.5, 'capsize': float(kwargs.get('capsize') or 0.0)}))
        bars.entry = None
        return container

    def hist(self, x, bins=10, range=None, density=False, weights=None, cumulative=False,
             bottom=None, histtype='bar', align='mid', orientation='vertical', rwidth=None,
             log=False, color=None, label=None, stacked=False, **kwargs):
        _check(kwargs, {'edgecolor', 'ec', 'linewidth', 'lw', 'alpha', 'facecolor', 'fill',
                        'linestyle', 'ls'}, 'hist')
        if histtype not in ('bar', 'barstacked', 'step', 'stepfilled'):
            raise ValueError("histtype is 'bar', 'barstacked', 'step' or 'stepfilled'")
        if align != 'mid':
            raise TypeError("gossia.pyplot does not support hist's align %r (only 'mid')" % (align,))
        if orientation not in ('vertical', 'horizontal'):
            raise ValueError("orientation is 'vertical' or 'horizontal'")
        data = PLAIN(x)
        if not isinstance(data, (list, tuple)):
            data = _list(data, 'the values of hist')
        many = bool(data) and all(isinstance(d, (list, tuple)) for d in data)
        sets = [list(d) for d in data] if many else [list(data)]
        sets = [[float(v) for v in d if v is not None and v == v] for d in sets]
        wsets = None
        if weights is not None:
            w = PLAIN(weights)
            wsets = [list(v) for v in w] if many else [list(w)]
        edges = _hist_edges(sets, bins, range)
        counts = [_hist_counts(d, edges, wsets[i] if wsets else None) for i, d in enumerate(sets)]
        widths = [b - a for a, b in zip(edges, edges[1:])]
        if density:
            counts = [[c / (sum(cs) * w) if sum(cs) else 0.0 for c, w in zip(cs, widths)]
                      for cs in counts]
        if cumulative:
            out = []
            for cs in counts:
                total, acc = 0.0, []
                for c, w in zip(cs, widths):
                    total += c * w if density else c
                    acc.append(total)
                out.append(acc)
            counts = out
        stacked = stacked or histtype == 'barstacked'
        if stacked:
            tops, run = [], [0.0] * len(widths)
            for cs in counts:
                run = [a + b for a, b in zip(run, cs)]
                tops.append(list(run))
        else:
            tops = counts
        names = [label] if not many or isinstance(label, str) or label is None else list(label)
        if many and isinstance(label, str):
            names = [label] + [None] * (len(sets) - 1)
        colours = PLAIN(color)
        if colours is None:
            colours = [self._next_colour() for _ in sets]
        elif isinstance(colours, (list, tuple)) and len(sets) > 1 and not (
                len(colours) in (3, 4) and all(isinstance(v, (int, float)) for v in colours)):
            colours = [_colour(c) for c in colours]
        else:
            colours = [_colour(colours)] * len(sets)
        vertical = orientation == 'vertical'
        alpha = kwargs.get('alpha')
        base = 0.0 if bottom is None else float(bottom)
        result = []
        if histtype in ('bar', 'barstacked'):
            nx = len(sets)
            dr = float(rwidth) if rwidth is not None else (0.8 if nx > 1 and not stacked else 1.0)
            for i, top in enumerate(tops):
                patches = []
                for j, w in enumerate(widths):
                    if stacked:
                        thick, offset = dr * w, (1 - dr) * w / 2.0
                        low = base + (tops[i - 1][j] if i else 0.0)
                    else:
                        thick = dr * w / nx
                        offset = (1 - dr) * w / 2.0 + i * thick
                        low = base
                    patches.append(_Bar({'facecolor': colours[i],
                                         'edgecolor': _colour(kwargs.get('edgecolor', kwargs.get('ec'))),
                                         'linewidth': float(kwargs.get('linewidth', kwargs.get('lw', 1.0))),
                                         'alpha': None if alpha is None else float(alpha)},
                                        edges[j] + offset, thick, low, top[j] - (low - base), vertical))
                bars = self._add(_Bars(patches, vertical))
                if names and i < len(names) and names[i] is not None and patches:
                    patches[0].style['label'] = str(names[i])
                    bars.entry = patches[0]
                result.append(_BarContainer(patches, None))
        else:
            filled = histtype == 'stepfilled'
            for i, top in enumerate(tops):
                low = [base + (tops[i - 1][j] if stacked and i else 0.0) for j in _range(len(widths))]
                xs = [edges[0]]
                ys = [low[0]]
                for j in _range(len(widths)):
                    xs += [edges[j], edges[j + 1]]
                    ys += [top[j] + base, top[j] + base]
                xs.append(edges[-1])
                ys.append(low[-1])
                if not vertical:
                    xs, ys = ys, xs
                style = {'color': colours[i], 'facecolor': colours[i] if filled else 'none',
                         'edgecolor': colours[i] if not filled else _colour(kwargs.get('edgecolor')),
                         'linewidth': float(kwargs.get('linewidth', kwargs.get('lw', 1.5 if not filled else 0.0))),
                         'alpha': None if alpha is None else float(alpha), 'closed': filled,
                         'label': None if i >= len(names) or names[i] is None else str(names[i])}
                poly = self._add(_Polygon([(xs, ys)], style, sticky_value=base, vertical=vertical))
                result.append(poly)
        if log:
            (self.yaxis if vertical else self.xaxis).scale = 'log'
        (self.xaxis if vertical else self.yaxis)._kind('number')
        (self.yaxis if vertical else self.xaxis)._kind('number')
        n = counts if many else counts[0]
        return n, edges, (result if many else result[0])

    def fill_between(self, x, y1, y2=0, where=None, interpolate=False, step=None, **kwargs):
        return self._fill(x, y1, y2, where, step, kwargs, True)

    def fill_betweenx(self, y, x1, x2=0, where=None, step=None, interpolate=False, **kwargs):
        return self._fill(y, x1, x2, where, step, kwargs, False)

    def _fill(self, ind, d1, d2, where, step, kwargs, vertical):
        name = 'fill_between' if vertical else 'fill_betweenx'
        _check(kwargs, {'color', 'facecolor', 'fc', 'edgecolor', 'ec', 'alpha', 'label',
                        'linewidth', 'lw', 'linestyle', 'ls'}, name)
        iaxis, daxis = (self.xaxis, self.yaxis) if vertical else (self.yaxis, self.xaxis)
        xs = iaxis.convert(ind, 'the positions')
        n = len(xs)
        a = daxis.convert(d1 if isinstance(PLAIN(d1), (list, tuple)) else [d1] * n, 'the values')
        b = daxis.convert(d2 if isinstance(PLAIN(d2), (list, tuple)) else [d2] * n, 'the values')
        if len(a) != n or len(b) != n:
            raise ValueError('%s: the positions and the values must have the same length' % name)
        mask = [True] * n if where is None else [bool(w) for w in _list(where, 'where')]
        if step not in (None, 'pre', 'post', 'mid'):
            raise ValueError("step is 'pre', 'post' or 'mid'")
        parts = []
        run = []
        for i in range(n):
            if mask[i] and xs[i] is not None and a[i] is not None and b[i] is not None:
                run.append(i)
            elif run:
                parts.append(run)
                run = []
        if run:
            parts.append(run)
        polys = []
        for run in parts:
            top = [(xs[i], a[i]) for i in run]
            low = [(xs[i], b[i]) for i in run]
            if step:
                top, low = _steps(top, step), _steps(low, step)
            pts = top + low[::-1]
            px = [p[0] for p in pts]
            py = [p[1] for p in pts]
            polys.append((px, py) if vertical else (py, px))
        colour = kwargs.get('color', kwargs.get('facecolor', kwargs.get('fc')))
        face = _colour(colour) if colour is not None else self._next_colour()
        edge = kwargs.get('edgecolor', kwargs.get('ec'))
        style = {'color': face, 'facecolor': face, 'edgecolor': _colour(edge),
                 'linewidth': float(kwargs.get('linewidth', kwargs.get('lw', 1.0 if edge else 0.0))),
                 'dash': _dash(kwargs.get('linestyle', kwargs.get('ls'))),
                 'alpha': None if kwargs.get('alpha') is None else float(kwargs['alpha']),
                 'label': None if kwargs.get('label') is None else str(kwargs['label'])}
        return self._add(_Polygon(polys, style))

    def errorbar(self, x, y, yerr=None, xerr=None, fmt='', ecolor=None, elinewidth=None,
                 capsize=None, barsabove=False, errorevery=1, capthick=None, **kwargs):
        _check(kwargs, self._LINE_ARGS | {'uplims', 'lolims', 'xuplims', 'xlolims'}, 'errorbar')
        for key in ('uplims', 'lolims', 'xuplims', 'xlolims'):
            if kwargs.pop(key, False):
                raise TypeError('gossia.pyplot does not support the argument %s of errorbar' % key)
        label = kwargs.pop('label', None)
        xs, ys = self._xy(x, y, 'errorbar')
        if fmt == 'none':
            color, marker, line = None, None, 'None'
        else:
            color, marker, line = _parse_format(fmt) if fmt else (None, None, None)
        style = self._line_style(kwargs, color, marker, line)
        if fmt == 'none':
            style['marker'] = None
        main = self._add(_Line(xs, ys, style))
        every = int(errorevery)
        keep = [i % every == 0 for i in range(len(xs))]
        bars = self._add(_ErrorBars(
            xs, ys, _errors(xerr, len(xs), 'xerr', keep), _errors(yerr, len(xs), 'yerr', keep),
            {'color': _colour(ecolor) if ecolor is not None else style['color'],
             'linewidth': float(elinewidth) if elinewidth is not None else style['linewidth'],
             'capsize': float(capsize) if capsize is not None else 0.0,
             'capthick': float(capthick) if capthick is not None else None,
             'alpha': style.get('alpha')}))
        container = _ErrorbarContainer(main, bars, label, xerr is not None, yerr is not None)
        self.containers.append(container)
        return container

    def boxplot(self, x, notch=None, sym=None, vert=None, whis=1.5, positions=None,
                widths=None, patch_artist=False, labels=None, tick_labels=None,
                showmeans=False, showfliers=True, showcaps=True, showbox=True,
                orientation='vertical', manage_ticks=True, boxprops=None, medianprops=None,
                whiskerprops=None, capprops=None, flierprops=None, meanprops=None,
                meanline=False, **kwargs):
        _check(kwargs, set(), 'boxplot')
        if notch:
            raise TypeError('gossia.pyplot does not support notched box plots')
        if isinstance(whis, (tuple, list)):
            raise TypeError('gossia.pyplot does not support whis as percentiles; give a number')
        data = PLAIN(x)
        data = list(data) if isinstance(data, (list, tuple)) else _list(data, 'the values of boxplot')
        sets = [list(d) for d in data] if data and all(isinstance(d, (list, tuple)) for d in data) else [data]
        sets = [sorted(float(v) for v in d if v is not None and v == v) for d in sets]
        n = len(sets)
        vertical = (vert if vert is not None else orientation == 'vertical')
        positions = [float(p) for p in _list(positions, 'positions')] if positions is not None \
            else [float(i + 1) for i in range(n)]
        if len(positions) != n:
            raise ValueError('boxplot: there are %d positions for %d boxes' % (len(positions), n))
        if widths is None:
            spread = max(positions) - min(positions) if n > 1 else 0.0
            widths = min(0.5, max(0.15, 0.15 * spread))
        widths = _per_point(widths, n, 'widths', float, None)
        names = tick_labels if tick_labels is not None else labels
        paxis, vaxis = (self.xaxis, self.yaxis) if vertical else (self.yaxis, self.xaxis)
        paxis._kind('number')
        vaxis._kind('number')
        props = {
            'box': _props(boxprops, {'color': 'currentColor', 'linewidth': 1.0,
                                     'facecolor': _CYCLE[0] if patch_artist else 'none'}),
            'median': _props(medianprops, {'color': _CYCLE[1], 'linewidth': 1.0}),
            'whisker': _props(whiskerprops, {'color': 'currentColor', 'linewidth': 1.0}),
            'cap': _props(capprops, {'color': 'currentColor', 'linewidth': 1.0}),
            'flier': _props(flierprops, {'marker': 'o' if sym is None else (sym or None),
                                         'markerfacecolor': 'none',
                                         'markeredgecolor': 'currentColor', 'markersize': 6.0}),
            'mean': _props(meanprops, {'marker': '^', 'markerfacecolor': _CYCLE[2],
                                       'markeredgecolor': _CYCLE[2], 'markersize': 6.0,
                                       'color': _CYCLE[2], 'linewidth': 1.0}),
        }
        boxes = []
        for i, values in enumerate(sets):
            stats = _box_stats(values, float(whis))
            boxes.append(_Box(positions[i], widths[i], stats, vertical,
                              {k: dict(v) for k, v in props.items()},
                              showmeans, meanline, showfliers, showcaps, showbox))
        self._add(_BoxPlot(boxes, vertical))
        if manage_ticks:
            paxis.ticks = list(positions)
            paxis.tick_labels = [str(l) for l in _list(names, 'tick_labels')] if names is not None \
                else [_plain_number(p) for p in positions]
            if len(paxis.tick_labels) != n:
                raise ValueError('boxplot: there are %d labels for %d boxes' % (len(paxis.tick_labels), n))
        return {'boxes': [b.handles['box'] for b in boxes],
                'medians': [b.handles['median'] for b in boxes],
                'whiskers': [h for b in boxes for h in (b.handles['whisker'],) * 2],
                'caps': [h for b in boxes for h in (b.handles['cap'],) * 2],
                'fliers': [b.handles['flier'] for b in boxes],
                'means': [b.handles['mean'] for b in boxes] if showmeans else []}

    def pie(self, x, explode=None, labels=None, colors=None, autopct=None, pctdistance=0.6,
            shadow=False, labeldistance=1.1, startangle=0, radius=1, counterclock=True,
            wedgeprops=None, textprops=None, center=(0, 0), frame=False, rotatelabels=False,
            normalize=True, **kwargs):
        _check(kwargs, set(), 'pie')
        if shadow:
            raise TypeError('gossia.pyplot does not support shadows on a pie')
        if rotatelabels:
            raise TypeError('gossia.pyplot does not support rotatelabels')
        values = [float(v) for v in _list(x, 'the values of pie')]
        if any(v < 0 for v in values):
            raise ValueError('the values of a pie cannot be negative')
        total = sum(values)
        if normalize:
            if total <= 0:
                raise ValueError('the values of a pie add up to 0')
            fracs = [v / total for v in values]
        else:
            if total > 1:
                raise ValueError('with normalize=False the values of a pie add up to 1 at most')
            fracs = values
        n = len(values)
        explode = _per_point(explode, n, 'explode', float, 0.0)
        names = [None] * n if labels is None else [str(l) for l in _list(labels, 'labels')]
        if len(names) != n:
            raise ValueError('pie: there are %d labels for %d values' % (len(names), n))
        if colors is None:
            faces = [self._next_colour() for _ in range(n)]
        else:
            given = [_colour(c) for c in _list(colors, 'colors')]
            faces = [given[i % len(given)] for i in range(n)]
        wp = dict(PLAIN(wedgeprops) or {})
        tp = dict(PLAIN(textprops) or {})
        for key in wp:
            if key not in ('edgecolor', 'ec', 'linewidth', 'lw', 'width', 'alpha', 'linestyle', 'ls'):
                raise TypeError('gossia.pyplot does not support %s in wedgeprops' % key)
        for key in tp:
            if key not in ('fontsize', 'size', 'color', 'c', 'fontweight', 'weight'):
                raise TypeError('gossia.pyplot does not support %s in textprops' % key)
        cx, cy = [float(v) for v in center]
        wedges = []
        texts = []
        autotexts = []
        theta = float(startangle)
        sign = 1.0 if counterclock else -1.0
        for i in range(n):
            span = 360.0 * fracs[i] * sign
            t1, t2 = theta, theta + span
            mid = _math.radians((t1 + t2) / 2.0)
            ox = explode[i] * _math.cos(mid)
            oy = explode[i] * _math.sin(mid)
            style = {'facecolor': faces[i], 'edgecolor': _colour(wp.get('edgecolor', wp.get('ec'))),
                     'linewidth': float(wp.get('linewidth', wp.get('lw', 0.0 if not wp.get('edgecolor', wp.get('ec')) else 1.0))),
                     'alpha': None if wp.get('alpha') is None else float(wp['alpha']),
                     'label': names[i]}
            width = wp.get('width')
            wedges.append(_Wedge(style, cx + ox, cy + oy, float(radius), min(t1, t2), max(t1, t2),
                                 None if width is None else float(width)))
            text_style = {'fontsize': _font_size(tp.get('fontsize', tp.get('size')), 10.0),
                          'color': _colour(tp.get('color', tp.get('c'))),
                          'fontweight': tp.get('fontweight', tp.get('weight'))}
            if names[i] is not None and labeldistance is not None:
                lx = cx + ox + labeldistance * radius * _math.cos(mid)
                ly = cy + oy + labeldistance * radius * _math.sin(mid)
                s = dict(text_style, text=names[i], ha='left' if lx > cx else 'right', va='center')
                texts.append(_Text(lx, ly, s))
            if autopct is not None:
                pct = fracs[i] * 100.0
                if callable(autopct):
                    label = str(autopct(pct))
                elif isinstance(autopct, str):
                    label = autopct % pct
                else:
                    raise TypeError('autopct is a format such as "%1.1f%%" or a function')
                px = cx + ox + pctdistance * radius * _math.cos(mid)
                py = cy + oy + pctdistance * radius * _math.sin(mid)
                autotexts.append(_Text(px, py, dict(text_style, text=label, ha='center', va='center')))
            theta = t2
        self.xaxis._kind('number')
        self.yaxis._kind('number')
        self._add(_Pie(wedges, texts + autotexts))
        self.aspect = 1.0
        if not frame:
            self.frame_on = False
            self.xaxis.ticks, self.xaxis.tick_labels = [], None
            self.yaxis.ticks, self.yaxis.tick_labels = [], None
            self.xaxis.limits = [cx - 1.25, cx + 1.25]
            self.yaxis.limits = [cy - 1.25, cy + 1.25]
        if autopct is not None:
            return wedges, texts, autotexts
        return wedges, texts

    def axhline(self, y=0, xmin=0, xmax=1, **kwargs):
        return self._axline(y, xmin, xmax, kwargs, True)

    def axvline(self, x=0, ymin=0, ymax=1, **kwargs):
        return self._axline(x, ymin, ymax, kwargs, False)

    def _axline(self, value, start, end, kwargs, horizontal):
        name = 'axhline' if horizontal else 'axvline'
        _check(kwargs, {'color', 'c', 'linestyle', 'ls', 'linewidth', 'lw', 'alpha', 'label'}, name)
        axis = self.yaxis if horizontal else self.xaxis
        v = axis.one(value, 'the position of %s' % name)
        # matplotlib draws these in the first colour without moving the cycle.
        style = self._line_style(kwargs, default_colour=_CYCLE[0])
        return self._add(_SpanLine(v, float(start), float(end), horizontal, style))

    def axhspan(self, ymin, ymax, xmin=0, xmax=1, **kwargs):
        return self._axspan(ymin, ymax, xmin, xmax, kwargs, True)

    def axvspan(self, xmin, xmax, ymin=0, ymax=1, **kwargs):
        return self._axspan(xmin, xmax, ymin, ymax, kwargs, False)

    def world(self, borders=True, land=None, edgecolor=None, linewidth=0.6, alpha=None):
        """Gossia's own: the world's land and country borders (Natural Earth,
        1:110m) behind what the Axes has, in degrees of longitude (x) and
        latitude (y), with an equal aspect."""
        if land is None:
            face, face_opacity = 'currentColor', 0.08
        elif str(land).strip().lower() == 'none':
            face, face_opacity = 'none', None
        else:
            face, face_opacity = _colour(land, 'land colour'), None
        if alpha is not None:
            face_opacity = float(alpha)
        edge = _colour(edgecolor, 'edge colour') if edgecolor is not None else 'currentColor'
        default = edgecolor is None
        style = {
            'borders': bool(borders),
            'land': face,
            'land_opacity': '' if face_opacity is None else ' fill-opacity="%s"' % _fmt(face_opacity),
            'edge': edge,
            'linewidth': float(linewidth),
            'edge_opacity': ' stroke-opacity="0.45"' if default else '',
            'border_opacity': ' stroke-opacity="0.3"' if default else ' stroke-opacity="0.6"',
        }
        artist = _World(style)
        # Behind everything else.
        self.artists.insert(0, artist)
        if self.aspect is None:
            self.aspect = 1.0
        return artist

    def _axspan(self, lo, hi, start, end, kwargs, horizontal):
        name = 'axhspan' if horizontal else 'axvspan'
        _check(kwargs, {'color', 'facecolor', 'fc', 'edgecolor', 'ec', 'alpha', 'label',
                        'linewidth', 'lw'}, name)
        axis = self.yaxis if horizontal else self.xaxis
        a, b = axis.one(lo, 'a limit of %s' % name), axis.one(hi, 'a limit of %s' % name)
        colour = kwargs.get('color', kwargs.get('facecolor', kwargs.get('fc')))
        face = _colour(colour) if colour is not None else _CYCLE[0]
        style = {'facecolor': face, 'color': face, 'edgecolor': _colour(kwargs.get('edgecolor', kwargs.get('ec'))),
                 'linewidth': float(kwargs.get('linewidth', kwargs.get('lw', 0.0))),
                 'alpha': None if kwargs.get('alpha') is None else float(kwargs['alpha']),
                 'label': None if kwargs.get('label') is None else str(kwargs['label'])}
        return self._add(_Span(a, b, float(start), float(end), horizontal, style))

    _TEXT_ARGS = frozenset(['fontsize', 'size', 'color', 'c', 'ha', 'horizontalalignment',
                            'va', 'verticalalignment', 'rotation', 'fontweight', 'weight',
                            'alpha', 'fontstyle', 'style', 'transform', 'fontdict'])

    def _text_style(self, kwargs, where):
        _check(kwargs, self._TEXT_ARGS, where)
        fontdict = dict(PLAIN(kwargs.pop('fontdict', None) or {}))
        for key in fontdict:
            if key not in self._TEXT_ARGS:
                raise TypeError('gossia.pyplot does not support %s in fontdict' % key)
        merged = dict(fontdict, **kwargs)
        ha = merged.get('ha', merged.get('horizontalalignment', 'left'))
        va = merged.get('va', merged.get('verticalalignment', 'baseline'))
        if ha not in ('left', 'center', 'right'):
            raise ValueError("ha is 'left', 'center' or 'right'")
        if va not in ('baseline', 'bottom', 'center', 'top', 'center_baseline'):
            raise ValueError("va is 'baseline', 'bottom', 'center' or 'top'")
        rotation = merged.get('rotation', 0.0)
        rotation = {'horizontal': 0.0, 'vertical': 90.0}.get(rotation, rotation)
        style = {'fontsize': _font_size(merged.get('fontsize', merged.get('size')), 10.0),
                 'color': _colour(merged.get('color', merged.get('c'))),
                 'ha': ha, 'va': va, 'rotation': float(rotation),
                 'fontweight': merged.get('fontweight', merged.get('weight')),
                 'fontstyle': merged.get('fontstyle', merged.get('style')),
                 'alpha': None if merged.get('alpha') is None else float(merged['alpha'])}
        transform = merged.get('transform', 'data')
        if transform not in ('data', 'axes fraction'):
            raise TypeError('gossia.pyplot takes transform=ax.transAxes or ax.transData only')
        return style, transform

    def text(self, x, y, s, **kwargs):
        style, transform = self._text_style(kwargs, 'text')
        style['text'] = str(s)
        if transform == 'data':
            x, y = self.xaxis.one(x, 'x'), self.yaxis.one(y, 'y')
        return self._add(_Text(x, y, style, transform))

    def annotate(self, text, xy, xytext=None, xycoords='data', textcoords=None,
                 arrowprops=None, **kwargs):
        style, _ = self._text_style(kwargs, 'annotate')
        style['text'] = str(text)
        coords = ('data', 'axes fraction', 'offset points', 'figure fraction')
        if xycoords not in coords[:2]:
            raise TypeError("gossia.pyplot takes xycoords 'data' or 'axes fraction'")
        textcoords = xycoords if textcoords is None else textcoords
        if textcoords not in coords[:3]:
            raise TypeError("gossia.pyplot takes textcoords 'data', 'axes fraction' or 'offset points'")
        point = self._coords(xy, xycoords)
        place = point if xytext is None else (
            tuple(float(v) for v in xytext) if textcoords == 'offset points'
            else self._coords(xytext, textcoords))
        arrow = None
        if arrowprops is not None:
            props = dict(PLAIN(arrowprops))
            for key in props:
                if key not in ('arrowstyle', 'color', 'facecolor', 'edgecolor', 'linewidth', 'lw',
                               'shrink', 'width', 'headwidth', 'headlength', 'connectionstyle',
                               'alpha', 'shrinkA', 'shrinkB'):
                    raise TypeError('gossia.pyplot does not support %s in arrowprops' % key)
            style_name = props.get('arrowstyle', '-|>' if 'arrowstyle' not in props else '->')
            if style_name not in ('-', '->', '-|>', '<-', '<|-', '<->', '<|-|>'):
                raise TypeError('gossia.pyplot does not support the arrowstyle %r' % (style_name,))
            cs = props.get('connectionstyle')
            if cs not in (None, 'arc3', 'arc3,rad=0'):
                raise TypeError('gossia.pyplot does not support the connectionstyle %r' % (cs,))
            # Without an arrowstyle, matplotlib draws its older, heavier arrow.
            old = 'arrowstyle' not in props
            arrow = {'style': style_name, 'old': old,
                     'color': _colour(props.get('color', props.get('facecolor', props.get('edgecolor')))),
                     'linewidth': float(props.get('linewidth', props.get('lw', props.get(
                         'width', 4.0 if old else 1.0)))),
                     'headwidth': float(props.get('headwidth', 12.0)),
                     'headlength': float(props.get('headlength', 12.0)),
                     'shrink': float(props.get('shrink', 0.0)),
                     'alpha': props.get('alpha')}
        return self._add(_Annotation(point, xycoords, place, textcoords, style, arrow))

    def _coords(self, xy, system):
        x, y = PLAIN(xy)
        if system == 'data':
            return (self.xaxis.one(x, 'x'), self.yaxis.one(y, 'y'))
        return (float(x), float(y))

    def legend(self, *args, **kwargs):
        _check(kwargs, {'loc', 'fontsize', 'frameon', 'ncol', 'ncols', 'title', 'title_fontsize',
                        'framealpha', 'facecolor', 'edgecolor', 'fancybox', 'handles', 'labels',
                        'markerscale', 'numpoints', 'scatterpoints', 'shadow', 'labelcolor'}, 'legend')
        if kwargs.get('shadow'):
            raise TypeError('gossia.pyplot does not support shadows on a legend')
        handles = kwargs.get('handles')
        labels = kwargs.get('labels')
        if len(args) == 1:
            labels = args[0]
        elif len(args) == 2:
            handles, labels = args
        elif args:
            raise TypeError('legend takes handles and labels, or labels')
        if handles is not None:
            handles = list(handles)
            labels = [h.get_label() for h in handles] if labels is None else [str(l) for l in labels]
        elif labels is not None:
            labels = [str(l) for l in _list(labels, 'the labels')]
            handles = self._legend_handles(all_of_them=True)[:len(labels)]
        else:
            handles = self._legend_handles()
            labels = [h.get_label() for h in handles]
        entries = [(h, l) for h, l in zip(handles, labels) if l and not l.startswith('_')]
        loc = kwargs.get('loc', 'best')
        if isinstance(loc, int) and not isinstance(loc, bool):
            if not 0 <= loc < len(_LEGEND_CODES):
                raise ValueError('a legend loc code is from 0 to 10')
            loc = _LEGEND_CODES[loc]
        if loc not in _LEGEND_CODES:
            raise TypeError('gossia.pyplot does not support the legend loc %r' % (loc,))
        size = kwargs.get('fontsize', 10.0)
        size = _FONT_SIZES.get(size, size)
        title_size = kwargs.get('title_fontsize', size)
        self.legend_ = _Legend(entries, {
            'loc': loc, 'fontsize': float(size),
            'frameon': kwargs.get('frameon', True) is not False,
            'ncol': int(kwargs.get('ncols', kwargs.get('ncol', 1))),
            'title': None if kwargs.get('title') is None else str(kwargs['title']),
            'title_fontsize': float(_FONT_SIZES.get(title_size, title_size)),
            'framealpha': float(kwargs.get('framealpha', 0.8) if kwargs.get('framealpha') is not None else 0.8),
            'facecolor': _colour(kwargs['facecolor']) if kwargs.get('facecolor') not in (None, 'inherit') else None,
            'edgecolor': _colour(kwargs['edgecolor']) if kwargs.get('edgecolor') not in (None, 'inherit') else None,
            'markerscale': float(kwargs.get('markerscale', 1.0)),
            'labelcolor': kwargs.get('labelcolor'),
        })
        return self.legend_

    def get_lines(self):
        return [a for a in self.artists if isinstance(a, _Line)]

    def get_legend(self):
        return self.legend_

    def get_legend_handles_labels(self):
        handles = self._legend_handles()
        return handles, [h.get_label() for h in handles]

    def _legend_handles(self, all_of_them=False):
        """The handles a legend shows, in matplotlib's order: what was drawn,
        then the bar and error bar containers."""
        found = []
        for artist in self.artists:
            found.extend(artist.entries())
        found.extend(self.containers)
        if all_of_them:
            return found
        return [h for h in found if h.get_label() and not h.get_label().startswith('_')]

    def twinx(self):
        """A second Axes on the same plot box, sharing x, with its y axis on
        the right."""
        if self.host is not None:
            raise TypeError('gossia.pyplot does not support a twin of a twin')
        if self.twin is not None:
            return self.twin
        twin = Axes(self.figure, *self.cell)
        twin.host = self
        _share(twin, 'xaxis', self.xaxis)
        self.twin = twin
        self.figure.axes.append(twin)
        self.figure.current = twin
        return twin

    def set(self, **kwargs):
        """Several settings at once: ax.set(title=..., xlabel=..., ylim=...)."""
        setters = {'title': self.set_title, 'xlabel': self.set_xlabel, 'ylabel': self.set_ylabel,
                   'xlim': self.set_xlim, 'ylim': self.set_ylim, 'xscale': self.set_xscale,
                   'yscale': self.set_yscale, 'xticks': self.set_xticks, 'yticks': self.set_yticks,
                   'xticklabels': self.set_xticklabels, 'yticklabels': self.set_yticklabels,
                   'aspect': self.set_aspect, 'frame_on': self.set_frame_on}
        for key, value in kwargs.items():
            if key not in setters:
                raise _refuse('Axes.set(%s=...)' % key)
            setters[key](value)

    def set_aspect(self, aspect, adjustable=None, anchor=None, share=False):
        if aspect == 'auto':
            self.aspect = None
        elif aspect == 'equal':
            self.aspect = 1.0
        else:
            self.aspect = float(aspect)
            if self.aspect <= 0:
                raise ValueError('an aspect is above 0')

    def set_frame_on(self, on):
        self.frame_on = bool(on)

    def set_axis_off(self):
        self.axis_on = False

    def set_axis_on(self):
        self.axis_on = True

    def axis(self, arg=None, **kwargs):
        """axis('off'), axis('on'), axis('equal'), axis('scaled'), axis('tight'),
        or the limits as (xmin, xmax, ymin, ymax)."""
        _check(kwargs, {'xmin', 'xmax', 'ymin', 'ymax'}, 'axis')
        if isinstance(arg, bool):
            arg = 'on' if arg else 'off'
        if isinstance(arg, str):
            if arg == 'off':
                self.axis_on = False
            elif arg == 'on':
                self.axis_on = True
            elif arg in ('equal', 'scaled', 'image'):
                self.aspect = 1.0
            elif arg == 'square':
                self.aspect = 'square'
            elif arg == 'tight':
                for axis in (self.xaxis, self.yaxis):
                    axis.tight = True
            elif arg == 'auto':
                self.aspect = None
            else:
                raise ValueError('gossia.pyplot does not support axis(%r)' % arg)
        elif arg is not None:
            limits = [float(v) for v in arg]
            if len(limits) != 4:
                raise ValueError('axis takes (xmin, xmax, ymin, ymax)')
            self.set_xlim(limits[0], limits[1])
            self.set_ylim(limits[2], limits[3])
        if kwargs:
            self.set_xlim(kwargs.get('xmin'), kwargs.get('xmax'))
            self.set_ylim(kwargs.get('ymin'), kwargs.get('ymax'))
        return self.get_xlim() + self.get_ylim()

    # -- titles and labels

    def set_title(self, label, loc='center', fontsize=None, **kwargs):
        _check(kwargs, {'pad', 'fontweight', 'color'}, 'set_title')
        if loc not in ('center', 'left', 'right'):
            raise ValueError("the title's loc is 'center', 'left' or 'right'")
        self.titles[loc] = (str(label), float(fontsize) if fontsize else _TITLE_SIZE)

    def get_title(self, loc='center'):
        return self.titles.get(loc, ('', 0))[0]

    def set_xlabel(self, label, fontsize=None, **kwargs):
        _check(kwargs, {'labelpad', 'color', 'fontweight'}, 'set_xlabel')
        self.xaxis.label = str(label)
        if fontsize:
            self.xaxis.label_size = float(fontsize)

    def set_ylabel(self, label, fontsize=None, **kwargs):
        _check(kwargs, {'labelpad', 'color', 'fontweight'}, 'set_ylabel')
        self.yaxis.label = str(label)
        if fontsize:
            self.yaxis.label_size = float(fontsize)

    def get_xlabel(self):
        return self.xaxis.label or ''

    def get_ylabel(self):
        return self.yaxis.label or ''

    # -- limits, scales and ticks

    def _set_limits(self, axis, left, right, where):
        if isinstance(left, (tuple, list)) and right is None:
            if len(left) != 2:
                raise ValueError('%s takes two limits' % where)
            left, right = left
        if left is not None:
            axis.limits[0] = axis.one(left, 'a limit')
        if right is not None:
            axis.limits[1] = axis.one(right, 'a limit')
        return self._get_limits(axis)

    def _get_limits(self, axis):
        lo, hi = self._limits(axis)
        if axis.kind == 'date':
            return (_number_date(lo), _number_date(hi))
        return (lo, hi)

    def set_xlim(self, left=None, right=None, **kwargs):
        _check(kwargs, {'xmin', 'xmax'}, 'set_xlim')
        left = kwargs.get('xmin', left)
        right = kwargs.get('xmax', right)
        return self._set_limits(self.xaxis, left, right, 'set_xlim')

    def set_ylim(self, bottom=None, top=None, **kwargs):
        _check(kwargs, {'ymin', 'ymax'}, 'set_ylim')
        bottom = kwargs.get('ymin', bottom)
        top = kwargs.get('ymax', top)
        return self._set_limits(self.yaxis, bottom, top, 'set_ylim')

    def get_xlim(self):
        return self._get_limits(self.xaxis)

    def get_ylim(self):
        return self._get_limits(self.yaxis)

    def _set_scale(self, axis, value, where):
        if value not in ('linear', 'log', 'mercator'):
            raise ValueError("gossia.pyplot does not support the scale %r (only 'linear', 'log'"
                             " and 'mercator')" % (value,))
        if value == 'log' and axis.kind in ('date', 'category'):
            raise ValueError('an axis of dates or text cannot be logarithmic')
        if value == 'mercator' and (axis.kind in ('date', 'category') or axis.name != 'y'):
            raise ValueError("the scale 'mercator' is for latitudes, on the y axis")
        axis.scale = value

    def set_xscale(self, value, **kwargs):
        _check(kwargs, {'base'}, 'set_xscale')
        self._set_scale(self.xaxis, value, 'set_xscale')

    def set_yscale(self, value, **kwargs):
        _check(kwargs, {'base'}, 'set_yscale')
        self._set_scale(self.yaxis, value, 'set_yscale')

    def _set_ticks(self, axis, ticks, labels, kwargs, where):
        _check(kwargs, {'rotation', 'fontsize', 'minor', 'ha'}, where)
        if kwargs.get('minor'):
            raise TypeError('gossia.pyplot does not support minor ticks')
        if ticks is not None:
            axis.ticks = [t for t in axis.convert(ticks, 'the ticks') if t is not None]
            axis.tick_labels = None
        if labels is not None:
            labels = [str(l) for l in _list(labels, 'the tick labels')]
            if axis.ticks is None:
                raise ValueError('give the ticks as well as their labels')
            if len(labels) != len(axis.ticks):
                raise ValueError('there are %d tick labels for %d ticks'
                                 % (len(labels), len(axis.ticks)))
            axis.tick_labels = labels
        if 'rotation' in kwargs:
            rotation = kwargs['rotation']
            axis.rotation = {'vertical': 90.0, 'horizontal': 0.0}.get(rotation, rotation)
            axis.rotation = float(axis.rotation)
        if kwargs.get('fontsize'):
            axis.tick_size = float(kwargs['fontsize'])

    def set_xticks(self, ticks, labels=None, **kwargs):
        self._set_ticks(self.xaxis, ticks, labels, kwargs, 'set_xticks')

    def set_yticks(self, ticks, labels=None, **kwargs):
        self._set_ticks(self.yaxis, ticks, labels, kwargs, 'set_yticks')

    def set_xticklabels(self, labels, **kwargs):
        if self.xaxis.ticks is None:
            self.xaxis.ticks = [p for p, _ in self._ticks(self.xaxis, self._limits(self.xaxis), 300)]
        self._set_ticks(self.xaxis, None, labels, kwargs, 'set_xticklabels')

    def set_yticklabels(self, labels, **kwargs):
        if self.yaxis.ticks is None:
            self.yaxis.ticks = [p for p, _ in self._ticks(self.yaxis, self._limits(self.yaxis), 200)]
        self._set_ticks(self.yaxis, None, labels, kwargs, 'set_yticklabels')

    def get_xticks(self):
        return [p for p, _ in self._ticks(self.xaxis, self._limits(self.xaxis), 300)]

    def get_yticks(self):
        return [p for p, _ in self._ticks(self.yaxis, self._limits(self.yaxis), 200)]

    def tick_params(self, axis='both', **kwargs):
        _check(kwargs, {'labelsize', 'rotation', 'labelrotation', 'which'}, 'tick_params')
        for a in self._which(axis):
            if kwargs.get('labelsize'):
                a.tick_size = float(kwargs['labelsize'])
            rotation = kwargs.get('rotation', kwargs.get('labelrotation'))
            if rotation is not None:
                a.rotation = float(rotation)

    def _which(self, axis):
        if axis not in ('x', 'y', 'both'):
            raise ValueError("axis is 'x', 'y' or 'both'")
        return [a for a in (self.xaxis, self.yaxis) if axis in ('both', a.name)]

    def grid(self, visible=None, which='major', axis='both', **kwargs):
        _check(kwargs, {'color', 'linestyle', 'linewidth', 'alpha', 'ls', 'lw'}, 'grid')
        if which not in ('major', 'both'):
            raise TypeError('gossia.pyplot does not support minor grid lines')
        on = True if visible is None and not kwargs else visible
        if visible is None and not kwargs:
            on = not all(self.grid_on[a.name] for a in self._which(axis))
        for a in self._which(axis):
            self.grid_on[a.name] = bool(True if on is None else on)

    def invert_xaxis(self):
        self.xaxis.inverted = not self.xaxis.inverted

    def invert_yaxis(self):
        self.yaxis.inverted = not self.yaxis.inverted

    # -- placing values

    def _usable(self, axis, value):
        return value is not None and (axis.scale != 'log' or value > 0)

    def _limits(self, axis):
        """The axis's limits: the user's, or the data's with a margin."""
        lo, hi = axis.limits
        if lo is not None and hi is not None:
            return lo, hi
        found = []
        sticky = []
        # Every Axes that uses this axis: a twin, or subplots that share it.
        for owner in axis.owners:
            which = 0 if owner.xaxis is axis else 1
            for artist in owner.artists:
                b = artist.bounds(owner)
                if b is not None and b[which] is not None:
                    found.append(b[which])
                sticky.extend(artist.sticky()[which])
        if axis.kind == 'category' and axis.categories:
            dlo, dhi = 0.0, float(len(axis.categories) - 1)
        elif found:
            dlo = min(f[0] for f in found)
            dhi = max(f[1] for f in found)
        else:
            dlo, dhi = (1.0, 10.0) if axis.scale == 'log' else (0.0, 1.0)
        log = axis.scale == 'log'
        if log:
            dlo, dhi = _math.log10(dlo), _math.log10(dhi)
        if dlo == dhi:
            pad = abs(dlo) * 0.05 if dlo else 0.05
            if axis.kind == 'category':
                pad = 0.5
            elif axis.kind == 'date':
                pad = 1.0
            dlo, dhi = dlo - pad, dhi + pad
        margin = 0.0 if axis.tight else (dhi - dlo) * 0.05
        if axis.kind == 'category' and not axis.tight:
            margin = max(margin, 0.5)
        mlo = dlo - margin
        mhi = dhi + margin
        if log:
            mlo, mhi = 10 ** mlo, 10 ** mhi
            dlo, dhi = 10 ** dlo, 10 ** dhi
        for s in sticky:
            if dlo >= s > mlo:
                mlo = s
            if dhi <= s < mhi:
                mhi = s
        return (mlo if lo is None else lo), (mhi if hi is None else hi)

    def _pos(self, axis, v, box, clamp=False):
        """Where a value goes along one axis in the SVG; at or below 0 on a
        logarithmic axis it is None, or the edge if `clamp`."""
        if v is None:
            return None
        left, top, right, bottom = box
        view = self._view[0] if axis is self.xaxis else self._view[1]
        if axis.scale == 'log' and v <= 0:
            if not clamp:
                return None
            v = min(view)
        if axis is self.xaxis:
            return _scale(axis, v, view, left, right)
        return _scale(axis, v, view, bottom, top)

    def _at(self, x, y, coords, box):
        """A point given in data coordinates or as fractions of the box."""
        if coords == 'axes fraction':
            left, top, right, bottom = box
            return (left + (right - left) * float(x), bottom - (bottom - top) * float(y))
        return self._place(x, y, box)

    def _place(self, x, y, box):
        """Where a point goes in the SVG, or None if it cannot be drawn."""
        if not (self._usable(self.xaxis, x) and self._usable(self.yaxis, y)):
            return None
        left, top, right, bottom = box
        return (_scale(self.xaxis, x, self._view[0], left, right),
                _scale(self.yaxis, y, self._view[1], bottom, top))

    def _ticks(self, axis, limits, length):
        """The ticks of an axis, as (position, label) pairs, and its offset
        text ('×10⁶'), for an axis `length` points long."""
        lo, hi = min(limits), max(limits)
        if axis.ticks is not None:
            labels = axis.tick_labels
            if labels is None:
                auto = dict(_auto_ticks(axis, lo, hi, length)[0])
                labels = [auto.get(t) or _plain_label(axis, t) for t in axis.ticks]
            return list(zip(axis.ticks, labels))
        return _auto_ticks(axis, lo, hi, length)[0]

    def _offset(self, axis, limits, length):
        if axis.ticks is not None:
            return ''
        return _auto_ticks(axis, min(limits), max(limits), length)[1]

    def clear(self):
        self._reset()
        for name in ('xaxis', 'yaxis'):
            axis = getattr(self, name)
            if len(axis.owners) == 1:
                setattr(self, name, _Axis(axis.name, self))

    cla = clear


def _scale(axis, value, limits, start, end):
    lo, hi = limits
    if axis.inverted:
        lo, hi = hi, lo
    if axis.scale == 'log':
        value, lo, hi = _math.log10(value), _math.log10(lo), _math.log10(hi)
    elif axis.scale == 'mercator':
        value, lo, hi = _mercator(value), _mercator(lo), _mercator(hi)
    if hi == lo:
        return (start + end) / 2.0
    return start + (value - lo) / (hi - lo) * (end - start)


# ------------------------------------------------------------------ ticks

_STEPS = (1.0, 2.0, 2.5, 5.0, 10.0)


def _count(axis, length):
    """About how many ticks fit along an axis."""
    spacing = 60.0 if axis.name == 'x' else 36.0
    return max(3, min(10, int(length / spacing)))


def _linear(lo, hi, count):
    """Round ticks between lo and hi: the widest step that gives no more
    than about `count` of them, but never fewer than three, unless the
    finest step tried still gives fewer."""
    span = hi - lo
    if span <= 0:
        return [lo], 1.0
    power = 10.0 ** _math.floor(_math.log10(span / count))
    candidates = [m * power / 10.0 for m in _STEPS[:-1]] + [m * power for m in _STEPS]         + [m * power * 10.0 for m in _STEPS[1:]]
    chosen = None
    for step in candidates:
        first = int(_math.ceil(lo / step - 1e-9))
        last = int(_math.floor(hi / step + 1e-9))
        n = last - first + 1
        if n <= count + 1:
            if n < 3 and chosen is not None:
                break
            chosen = (step, first, last)
            break
        chosen = (step, first, last)
    step, first, last = chosen
    ticks = []
    for k in range(first, last + 1):
        t = round(k * step, 12)
        ticks.append(0.0 if t == 0 else t)
    return ticks, step


def _number_labels(ticks, step):
    """Labels for ticks a step apart, with as many decimals as they need,
    and the offset text when they are very large or small."""
    if not ticks:
        return [], ''
    big = max(abs(t) for t in ticks)
    exponent = 0
    if big >= 1e6 or 0 < big < 1e-3:
        exponent = int(_math.floor(_math.log10(big)))
    scale = 10.0 ** exponent
    values = [t / scale for t in ticks]
    step = step / scale
    decimals = max(0, -int(_math.floor(_math.log10(step) + 1e-9)))
    while decimals < 10 and any(abs(round(v, decimals) - v) > step * 1e-6 for v in values):
        decimals += 1
    labels = []
    for v in values:
        text = '%.*f' % (decimals, v)
        if float(text) == 0:
            text = text.lstrip('-')
        labels.append(text.replace('-', _MINUS))
    return labels, ('×10' + _superscript(exponent)) if exponent else ''


def _log_ticks(lo, hi, count):
    a = int(_math.floor(_math.log10(lo) + 1e-9))
    b = int(_math.ceil(_math.log10(hi) - 1e-9))
    decades = list(range(a, b + 1))
    every = max(1, int(_math.ceil(len(decades) / float(count))))
    ticks = []
    for e in decades:
        if e % every:
            continue
        t = 10.0 ** e
        if lo * (1 - 1e-9) <= t <= hi * (1 + 1e-9):
            ticks.append((t, '10' + _superscript(e)))
    if len(ticks) < 2:
        # Less than two decades shown: 2 and 5 times them too.
        ticks = []
        for e in range(a - 1, b + 1):
            for m in (1, 2, 5):
                t = m * 10.0 ** e
                if lo * (1 - 1e-9) <= t <= hi * (1 + 1e-9):
                    ticks.append((t, _plain_number(t)))
    return ticks


def _plain_number(t):
    if t == int(t) and abs(t) < 1e15:
        text = str(int(t))
    else:
        text = '%g' % t
    return text.replace('-', _MINUS)


def _plain_label(axis, t):
    if axis.kind == 'date':
        d = _number_date(t)
        return d.strftime('%Y-%m-%d %H:%M') if axis.datetime else d.date().isoformat()
    if axis.kind == 'category':
        for name, pos in axis.categories.items():
            if pos == t:
                return name
        return ''
    return _plain_number(t)


def _auto_ticks(axis, lo, hi, length):
    count = _count(axis, length)
    if axis.kind == 'category':
        names = sorted(axis.categories.items(), key=lambda item: item[1])
        shown = [(pos, name) for name, pos in names if lo - 1e-9 <= pos <= hi + 1e-9]
        if axis.name == 'x' and shown and axis.rotation == 0:
            widest = max(_width(n, axis.tick_size) for _, n in shown)
            room = length / max(len(shown), 1)
            every = max(1, int(_math.ceil((widest + 6) / max(room, 1))))
            shown = shown[::every]
        elif axis.name == 'y' and shown:
            every = max(1, int(_math.ceil(len(shown) * axis.tick_size * 1.4 / max(length, 1))))
            shown = shown[::every]
        return shown, ''
    if axis.kind == 'date':
        return _date_ticks(lo, hi, count), ''
    if axis.scale == 'log':
        return _log_ticks(lo, hi, count), ''
    if axis.scale == 'mercator':
        # Latitudes, past the cut only at the edge.
        lo, hi = max(lo, -_MERCATOR_LIMIT), min(hi, _MERCATOR_LIMIT)
    ticks, step = _linear(lo, hi, count)
    labels, offset = _number_labels(ticks, step)
    return list(zip(ticks, labels)), offset


_MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']
_DAY = 1.0
_INTERVALS = (
    [('second', n, n / 86400.0) for n in (1, 2, 5, 10, 15, 30)]
    + [('minute', n, n / 1440.0) for n in (1, 2, 5, 10, 15, 30)]
    + [('hour', n, n / 24.0) for n in (1, 2, 3, 4, 6, 12)]
    + [('day', n, float(n)) for n in (1, 2, 3, 7, 14)]
    + [('month', n, n * 30.44) for n in (1, 2, 3, 4, 6)]
    + [('year', n, n * 365.25) for n in (1, 2, 5, 10, 20, 25, 50, 100, 200, 250, 500, 1000)]
)


def _date_ticks(lo, hi, count):
    """Ticks for an axis of dates, on whole seconds, minutes, hours, days,
    months or years, with labels fitting the interval."""
    span = hi - lo
    unit, n = 'year', 1000
    for u, k, days in _INTERVALS:
        if span / days <= count:
            unit, n = u, k
            break
    start = _number_date(lo)
    end = _number_date(hi)
    ticks = []
    if unit in ('second', 'minute', 'hour'):
        size = {'second': 1, 'minute': 60, 'hour': 3600}[unit] * n
        day = _dt.datetime(start.year, start.month, start.day)
        offset = (start - day).days * 86400 + (start - day).seconds
        t = day + _dt.timedelta(seconds=int(_math.ceil(offset / float(size))) * size)
        while t <= end and len(ticks) < 100:
            ticks.append(t)
            t = t + _dt.timedelta(seconds=size)
    elif unit == 'day':
        t = _dt.datetime(start.year, start.month, start.day)
        if t < start:
            t = t + _dt.timedelta(days=1)
        while t <= end and len(ticks) < 100:
            # Days 1, 1 + n, 1 + 2n ... of each month.
            if (t.day - 1) % n == 0:
                ticks.append(t)
            t = t + _dt.timedelta(days=1)
    elif unit == 'month':
        y, m = start.year, start.month
        if _dt.datetime(y, m, 1) < start:
            m += 1
            if m > 12:
                y, m = y + 1, 1
        while len(ticks) < 100:
            t = _dt.datetime(y, m, 1)
            if t > end:
                break
            if (m - 1) % n == 0:
                ticks.append(t)
            m += 1
            if m > 12:
                y, m = y + 1, 1
    else:
        y = start.year
        if _dt.datetime(y, 1, 1) < start:
            y += 1
        y = int(_math.ceil(y / float(n)) * n)
        while y <= end.year and y <= 9999 and len(ticks) < 100:
            ticks.append(_dt.datetime(y, 1, 1))
            y += n
    out = []
    first = True
    for t in ticks:
        if unit == 'year':
            label = str(t.year)
        elif unit == 'month':
            label = _MONTHS[t.month - 1]
            if first or t.month == 1:
                label += ' %d' % t.year
        elif unit == 'day':
            label = '%d %s' % (t.day, _MONTHS[t.month - 1])
        else:
            label = '%02d:%02d' % (t.hour, t.minute)
            if unit == 'second':
                label += ':%02d' % t.second
            if first or (t.hour == 0 and t.minute == 0 and t.second == 0):
                label = '%d %s %s' % (t.day, _MONTHS[t.month - 1], label)
        first = False
        number = t.toordinal() + (t.hour * 3600 + t.minute * 60 + t.second) / 86400.0
        out.append((number, label))
    return out


# ----------------------------------------------------------------- figure


class Figure:
    """A chart: one or more Axes, drawn to SVG by plt.svg()."""

    def __init__(self, width, height):
        self.width = float(width)
        self.height = float(height)
        self.axes = []
        self.current = None
        self.title = None

    def __getattr__(self, name):
        raise _refuse('Figure.%s' % name)

    def add_subplot(self, nrows=1, ncols=1, index=1, **kwargs):
        _check(kwargs, set(), 'add_subplot')
        if isinstance(nrows, int) and nrows >= 111 and ncols == 1 and index == 1:
            nrows, ncols, index = nrows // 100, (nrows // 10) % 10, nrows % 10
        nrows, ncols, index = int(nrows), int(ncols), int(index)
        if nrows < 1 or ncols < 1 or not 1 <= index <= nrows * ncols:
            raise ValueError('a subplot is numbered from 1 to %d' % (nrows * ncols))
        row, col = divmod(index - 1, ncols)
        for ax in self.axes:
            if ax.cell == (row, col, nrows, ncols):
                self.current = ax
                return ax
        ax = Axes(self, row, col, nrows, ncols)
        self.axes.append(ax)
        self.current = ax
        return ax

    def subplots(self, nrows=1, ncols=1, *, sharex=False, sharey=False, squeeze=True, **kwargs):
        _check(kwargs, set(), 'subplots')
        nrows, ncols = int(nrows), int(ncols)
        if nrows < 1 or ncols < 1:
            raise ValueError('subplots needs at least one row and one column')
        grid = [[self.add_subplot(nrows, ncols, r * ncols + c + 1) for c in range(ncols)]
                for r in range(nrows)]
        # Shared axes: one _Axis object for the Axes that share it, and the
        # tick labels only on the outside, as matplotlib does.
        for name, how in (('xaxis', sharex), ('yaxis', sharey)):
            how = {True: 'all', False: 'none'}.get(how, how)
            if how not in ('all', 'row', 'col', 'none'):
                raise ValueError("share%s is True, False, 'all', 'row', 'col' or 'none'" % name[0])
            for r in range(nrows):
                for c in range(ncols):
                    if how == 'all':
                        first = grid[0][0]
                    elif how == 'row':
                        first = grid[r][0]
                    elif how == 'col':
                        first = grid[0][c]
                    else:
                        continue
                    if grid[r][c] is not first:
                        _share(grid[r][c], name, getattr(first, name))
            for r in range(nrows):
                for c in range(ncols):
                    if name == 'xaxis' and how in ('all', 'col') and r < nrows - 1:
                        grid[r][c].hide['x'] = True
                    if name == 'yaxis' and how in ('all', 'row') and c > 0:
                        grid[r][c].hide['y'] = True
        self.current = grid[0][0]
        if squeeze and nrows == 1 and ncols == 1:
            return grid[0][0]
        if squeeze and (nrows == 1 or ncols == 1):
            return AxesArray([ax for row in grid for ax in row])
        return AxesArray([AxesArray(row) for row in grid])

    def gca(self):
        if self.current is None:
            return self.add_subplot(1, 1, 1)
        return self.current

    def suptitle(self, t, fontsize=None, **kwargs):
        _check(kwargs, {'fontweight', 'color', 'y'}, 'suptitle')
        self.title = (str(t), float(fontsize) if fontsize else _SUPTITLE_SIZE)

    def tight_layout(self, *args, **kwargs):
        # Layout is always tight.
        return None

    def savefig(self, *args, **kwargs):
        raise _savefig()

    def clear(self):
        self.axes = []
        self.current = None
        self.title = None

    clf = clear


class AxesArray:
    """The Axes plt.subplots() gives for more than one: indexed `axs[0]`, or
    `axs[0, 1]` for rows and columns, and iterated."""

    def __init__(self, items):
        self._items = list(items)

    def __getitem__(self, key):
        if isinstance(key, tuple):
            item = self
            for k in key:
                item = item[k]
            return item
        return self._items[key]

    def __iter__(self):
        return iter(self._items)

    def __len__(self):
        return len(self._items)

    def _flat(self):
        out = []
        for item in self._items:
            out.extend(item._flat() if isinstance(item, AxesArray) else [item])
        return out

    @property
    def flat(self):
        return iter(self._flat())

    def ravel(self):
        return AxesArray(self._flat())

    flatten = ravel

    @property
    def shape(self):
        if self._items and isinstance(self._items[0], AxesArray):
            return (len(self._items), len(self._items[0]))
        return (len(self._items),)


# ----------------------------------------------------------------- layout


def _text(out, x, y, text, size, anchor='middle', weight=None, rotate=None, opacity=None):
    attrs = ''
    if weight:
        attrs += ' font-weight="%s"' % weight
    if rotate:
        attrs += ' transform="rotate(%s %s %s)"' % (_fmt(rotate), _fmt(x), _fmt(y))
    fill = 'currentColor'
    if opacity is not None:
        attrs += ' fill-opacity="%s"' % _fmt(opacity)
    out.append('<text x="%s" y="%s" font-size="%s" text-anchor="%s" fill="%s"%s>%s</text>'
               % (_fmt(x), _fmt(y), _fmt(size), anchor, fill, attrs, _esc(text)))


def _rotated_height(labels, size, rotation):
    if not labels:
        return 0.0
    if not rotation:
        return size * _LINE
    a = _math.radians(abs(rotation))
    widest = max(_width(l, size) for l in labels)
    return widest * _math.sin(a) + size * _math.cos(a)


def _span(axis, limits):
    lo, hi = limits
    if axis.scale == 'log' and lo > 0 and hi > 0:
        return abs(_math.log10(hi) - _math.log10(lo)) or 1.0
    if axis.scale == 'mercator':
        return abs(_mercator(hi) - _mercator(lo)) or 1.0
    return abs(hi - lo) or 1.0


def _layout(ax, cell):
    """The plot box of an Axes in its cell, with its ticks: three rounds,
    since the ticks depend on the box's size and the margins on the ticks."""
    x0, y0, x1, y1 = cell
    twin = ax.twin
    show = ax.axis_on
    tshow = twin is not None and twin.axis_on
    has_title = bool(ax.titles)
    title_h = max([s for _, s in ax.titles.values()] or [0]) * _LINE + 4 if has_title else 0.0
    xlabel_h = ax.xaxis.label_size * _LINE + 2 if ax.xaxis.label and show else 0.0
    ylabel_w = ax.yaxis.label_size * _LINE + 2 if ax.yaxis.label and show else 0.0
    tlabel_w = twin.yaxis.label_size * _LINE + 2 if tshow and twin.yaxis.label else 0.0
    xlim = ax._limits(ax.xaxis)
    ylim = ax._limits(ax.yaxis)
    tlim = twin._limits(twin.yaxis) if twin is not None else None
    room = _TICK_LEN + _TICK_GAP
    box = (x0 + _PAD + 30, y0 + _PAD + title_h, x1 - _PAD - 10, y1 - _PAD - 24)
    for _ in range(3):
        left, top, right, bottom = box
        xt = ax._ticks(ax.xaxis, xlim, right - left)
        yt = ax._ticks(ax.yaxis, ylim, bottom - top)
        tt = twin._ticks(twin.yaxis, tlim, bottom - top) if tshow else []
        xl = [l for _, l in xt] if show and not ax.hide['x'] else []
        yl = [l for _, l in yt] if show and not ax.hide['y'] else []
        tl = [l for _, l in tt]
        yoff = ax._offset(ax.yaxis, ylim, bottom - top) if show else ''
        toff = twin._offset(twin.yaxis, tlim, bottom - top) if tshow else ''
        ywide = max([_width(l, ax.yaxis.tick_size) for l in yl] or [0])
        twide = max([_width(l, twin.yaxis.tick_size) for l in tl] or [0]) if tshow else 0.0
        xhigh = _rotated_height(xl, ax.xaxis.tick_size, ax.xaxis.rotation)
        # Room for half of the last x label beyond the box, when it is not rotated.
        overhang = 0.0
        if xl and not ax.xaxis.rotation:
            last = xt[-1]
            pos = _scale(ax.xaxis, last[0], xlim, left, right) if ax._usable(ax.xaxis, last[0]) else right
            overhang = max(0.0, pos + _width(last[1], ax.xaxis.tick_size) / 2 - right)
        right_room = max(overhang, (twide + room + tlabel_w) if tshow else 0.0)
        top_extra = ax.yaxis.tick_size * _LINE if (yoff or toff) else 0.0
        box = (x0 + _PAD + ylabel_w + ywide + (room if show else 0.0),
               y0 + _PAD + max(title_h, top_extra),
               x1 - _PAD - right_room,
               y1 - _PAD - xlabel_h - xhigh - (room if show else 0.0))
    left, top, right, bottom = box
    if ax.aspect is not None and right > left and bottom > top:
        # An equal aspect makes the box smaller, centred in the room it had.
        w, h = right - left, bottom - top
        ratio = 1.0 if ax.aspect == 'square' else ax.aspect * _span(ax.yaxis, ylim) / _span(ax.xaxis, xlim)
        if h > w * ratio:
            nh = w * ratio
            top += (h - nh) / 2.0
            bottom = top + nh
        else:
            nw = h / ratio
            left += (w - nw) / 2.0
            right = left + nw
        box = (left, top, right, bottom)
    if right - left < 10 or bottom - top < 10:
        raise ValueError('the chart is too small for its labels')
    return {'box': box, 'xlim': xlim, 'ylim': ylim, 'tlim': tlim,
            'xt': ax._ticks(ax.xaxis, xlim, right - left),
            'yt': ax._ticks(ax.yaxis, ylim, bottom - top),
            'tt': twin._ticks(twin.yaxis, tlim, bottom - top) if tshow else []}


def _draw_axes(out, ax, cell, clip_id):
    lay = _layout(ax, cell)
    box, xlim, ylim = lay['box'], lay['xlim'], lay['ylim']
    ax._view = (xlim, ylim)
    twin = ax.twin
    if twin is not None:
        twin._view = (xlim, lay['tlim'])
    left, top, right, bottom = box
    x0, y0, x1, y1 = cell
    show = ax.axis_on
    grid = []
    ticks = []
    if show:
        for pos, label in lay['xt']:
            if not ax._usable(ax.xaxis, pos):
                continue
            x = _scale(ax.xaxis, pos, xlim, left, right)
            if x < left - 0.5 or x > right + 0.5:
                continue
            ticks.append('M%s %s V%s' % (_fmt(x), _fmt(bottom), _fmt(bottom + _TICK_LEN)))
            if ax.grid_on['x']:
                grid.append('M%s %s V%s' % (_fmt(x), _fmt(top), _fmt(bottom)))
            if ax.hide['x']:
                continue
            size = ax.xaxis.tick_size
            if ax.xaxis.rotation:
                _text(out, x, bottom + _TICK_LEN + _TICK_GAP + size * 0.5, label, size,
                      anchor='end', rotate=-ax.xaxis.rotation, opacity=0.8)
            else:
                _text(out, x, bottom + _TICK_LEN + _TICK_GAP + size * _ASCENT, label, size,
                      opacity=0.8)
        for pos, label in lay['yt']:
            if not ax._usable(ax.yaxis, pos):
                continue
            y = _scale(ax.yaxis, pos, ylim, bottom, top)
            if y < top - 0.5 or y > bottom + 0.5:
                continue
            ticks.append('M%s %s H%s' % (_fmt(left - _TICK_LEN), _fmt(y), _fmt(left)))
            if ax.grid_on['y']:
                grid.append('M%s %s H%s' % (_fmt(left), _fmt(y), _fmt(right)))
            if ax.hide['y']:
                continue
            size = ax.yaxis.tick_size
            _text(out, left - _TICK_LEN - _TICK_GAP, y + size * (_ASCENT / 2), label, size,
                  anchor='end', opacity=0.8)
        offset = ax._offset(ax.yaxis, ylim, bottom - top)
        if offset and not ax.hide['y']:
            _text(out, left, top - 3, offset, ax.yaxis.tick_size, anchor='start', opacity=0.8)
        xoffset = ax._offset(ax.xaxis, xlim, right - left)
        if xoffset and not ax.hide['x']:
            _text(out, right, bottom + _TICK_LEN + _TICK_GAP + ax.xaxis.tick_size * (_ASCENT + _LINE),
                  xoffset, ax.xaxis.tick_size, anchor='end', opacity=0.8)
    if twin is not None and twin.axis_on:
        tlim = lay['tlim']
        for pos, label in lay['tt']:
            if not twin._usable(twin.yaxis, pos):
                continue
            y = _scale(twin.yaxis, pos, tlim, bottom, top)
            if y < top - 0.5 or y > bottom + 0.5:
                continue
            ticks.append('M%s %s H%s' % (_fmt(right), _fmt(y), _fmt(right + _TICK_LEN)))
            if twin.grid_on['y']:
                grid.append('M%s %s H%s' % (_fmt(left), _fmt(y), _fmt(right)))
            size = twin.yaxis.tick_size
            _text(out, right + _TICK_LEN + _TICK_GAP, y + size * (_ASCENT / 2), label, size,
                  anchor='start', opacity=0.8)
        offset = twin._offset(twin.yaxis, tlim, bottom - top)
        if offset:
            _text(out, right, top - 3, offset, twin.yaxis.tick_size, anchor='end', opacity=0.8)
    if grid:
        out.append('<path d="%s" fill="none" stroke="currentColor" stroke-opacity="0.16"'
                   ' stroke-width="0.8"/>' % ' '.join(grid))
    # What is drawn, clipped to the box: the Axes, then its twin over it.
    layers = [ax] if twin is None else [ax, twin]
    out.append('<clipPath id="%s"><rect x="%s" y="%s" width="%s" height="%s"/></clipPath>'
               % (clip_id, _fmt(left), _fmt(top), _fmt(right - left), _fmt(bottom - top)))
    out.append('<g clip-path="url(#%s)">' % clip_id)
    for layer in layers:
        for artist in layer.artists:
            artist.draw(out, layer, box)
    out.append('</g>')
    # The frame and the ticks.
    if ticks:
        out.append('<path d="%s" fill="none" stroke="currentColor" stroke-opacity="0.8"'
                   ' stroke-width="0.8"/>' % ' '.join(ticks))
    if show and ax.frame_on:
        out.append('<rect x="%s" y="%s" width="%s" height="%s" fill="none" stroke="currentColor"'
                   ' stroke-opacity="0.8" stroke-width="0.8"/>'
                   % (_fmt(left), _fmt(top), _fmt(right - left), _fmt(bottom - top)))
    # Texts, which are not clipped.
    for layer in layers:
        for artist in layer.artists:
            artist.draw_over(out, layer, box)
    # Labels and titles.
    if show and ax.xaxis.label:
        size = ax.xaxis.label_size
        _text(out, (left + right) / 2, y1 - _PAD - size * (_LINE - _ASCENT), ax.xaxis.label, size)
    if show and ax.yaxis.label:
        size = ax.yaxis.label_size
        _text(out, x0 + _PAD + size * _ASCENT, (top + bottom) / 2, ax.yaxis.label, size, rotate=-90)
    if twin is not None and twin.axis_on and twin.yaxis.label:
        size = twin.yaxis.label_size
        _text(out, x1 - _PAD - size * 0.25, (top + bottom) / 2, twin.yaxis.label, size, rotate=-90)
    for loc, (title, size) in ax.titles.items():
        x = {'center': (left + right) / 2, 'left': left, 'right': right}[loc]
        anchor = {'center': 'middle', 'left': 'start', 'right': 'end'}[loc]
        _text(out, x, top - 6, title, size, anchor=anchor)
    for layer in layers:
        _draw_legend(out, layer, box)


def _render(fig):
    out = []
    top = 0.0
    if fig.title:
        text, size = fig.title
        top = _PAD + size * _LINE
        _text(out, fig.width / 2, _PAD + size * _ASCENT, text, size, weight='600')
    axes = [ax for ax in fig.axes if ax.host is None] or [Axes(fig, 0, 0, 1, 1)]
    for i, ax in enumerate(axes):
        row, col, rows, cols = ax.cell
        w = fig.width / cols
        h = (fig.height - top) / rows
        cell = (col * w, top + row * h, (col + 1) * w, top + (row + 1) * h)
        _draw_axes(out, ax, cell, '\x00clip%d' % i)
    body = ''.join(out)
    tag = 'gp' + _fnv(body)
    body = body.replace('\x00clip', tag + '-')
    return ('<svg xmlns="http://www.w3.org/2000/svg" width="%s" height="%s" viewBox="0 0 %s %s"'
            ' font-family="%s" color="%s">' % (_fmt(fig.width), _fmt(fig.height), _fmt(fig.width),
                                               _fmt(fig.height), _FONT, _INK)
            + '<rect width="100%%" height="100%%" fill="%s" class="background"/>' % _SURFACE
            + body + '</svg>')


# ---------------------------------------------------------------- pyplot


def _savefig():
    return TypeError('a chart function cannot write files: finish it with return plt.svg()')


def _chart_size(width, height):
    """Called by Gossia before each chart call: the size of the chart."""
    _state['size'] = (float(width), float(height))


def clear():
    """Start a chart: forget every figure left over."""
    _state['figures'] = []
    _state['current'] = None


def figure(num=None, figsize=None, width=None, height=None, **kwargs):
    """A new figure, the size of the chart unless given `width` and `height`
    in points; `figsize` is read as a ratio of width to height only."""
    _check(kwargs, {'facecolor', 'edgecolor', 'layout', 'clear'}, 'figure')
    if num is not None:
        raise TypeError('gossia.pyplot does not support numbered figures (the argument num)')
    w, h = _state['size']
    if width is not None:
        w = float(width)
    if height is not None:
        h = float(height)
    if figsize is not None and (width is None or height is None):
        fw, fh = [float(v) for v in figsize]
        if fw <= 0 or fh <= 0:
            raise ValueError('figsize is (width, height)')
        if height is None:
            h = w * fh / fw
        else:
            w = h * fw / fh
    if w <= 0 or h <= 0:
        raise ValueError('a figure needs a width and height above 0')
    fig = Figure(w, h)
    _state['figures'].append(fig)
    _state['current'] = fig
    return fig


def gcf():
    if _state['current'] is None:
        return figure()
    return _state['current']


def gca():
    return gcf().gca()


def subplots(nrows=1, ncols=1, *, sharex=False, sharey=False, squeeze=True, figsize=None,
             width=None, height=None, **kwargs):
    _check(kwargs, {'subplot_kw', 'gridspec_kw', 'layout'}, 'subplots')
    for key in ('subplot_kw', 'gridspec_kw'):
        if kwargs.get(key):
            raise TypeError('gossia.pyplot does not support the argument %s of subplots' % key)
    fig = figure(figsize=figsize, width=width, height=height)
    return fig, fig.subplots(nrows, ncols, sharex=sharex, sharey=sharey, squeeze=squeeze)


def subplot(*args, **kwargs):
    return gcf().add_subplot(*args, **kwargs)


def svg():
    """The SVG text of the current figure, which is then closed."""
    fig = _state['current']
    if fig is None:
        raise ValueError('there is no figure to draw: plot something before plt.svg()')
    text = _render(fig)
    close(fig)
    return text


def close(fig=None):
    figures = _state['figures']
    if fig == 'all':
        clear()
        return
    if fig is None:
        fig = _state['current']
    if fig in figures:
        figures.remove(fig)
    if _state['current'] is fig:
        _state['current'] = figures[-1] if figures else None


def plot(*args, **kwargs):
    return gca().plot(*args, **kwargs)


def step(x, y, *args, **kwargs):
    return gca().step(x, y, *args, **kwargs)


def scatter(x, y, *args, **kwargs):
    return gca().scatter(x, y, *args, **kwargs)


def bar(x, height, *args, **kwargs):
    return gca().bar(x, height, *args, **kwargs)


def barh(y, width, *args, **kwargs):
    return gca().barh(y, width, *args, **kwargs)


def hist(x, *args, **kwargs):
    return gca().hist(x, *args, **kwargs)


def pie(x, *args, **kwargs):
    return gca().pie(x, *args, **kwargs)


def fill_between(x, y1, *args, **kwargs):
    return gca().fill_between(x, y1, *args, **kwargs)


def fill_betweenx(y, x1, *args, **kwargs):
    return gca().fill_betweenx(y, x1, *args, **kwargs)


def errorbar(x, y, *args, **kwargs):
    return gca().errorbar(x, y, *args, **kwargs)


def boxplot(x, *args, **kwargs):
    return gca().boxplot(x, *args, **kwargs)


def axhline(*args, **kwargs):
    return gca().axhline(*args, **kwargs)


def axvline(*args, **kwargs):
    return gca().axvline(*args, **kwargs)


def axhspan(*args, **kwargs):
    return gca().axhspan(*args, **kwargs)


def axvspan(*args, **kwargs):
    return gca().axvspan(*args, **kwargs)


def text(x, y, s, **kwargs):
    return gca().text(x, y, s, **kwargs)


def annotate(text, xy, *args, **kwargs):
    return gca().annotate(text, xy, *args, **kwargs)


def legend(*args, **kwargs):
    return gca().legend(*args, **kwargs)


def twinx(ax=None):
    return (ax or gca()).twinx()


def axis(*args, **kwargs):
    return gca().axis(*args, **kwargs)


def world(*args, **kwargs):
    return gca().world(*args, **kwargs)


def title(label, loc='center', fontsize=None, **kwargs):
    return gca().set_title(label, loc=loc, fontsize=fontsize, **kwargs)


def suptitle(t, **kwargs):
    return gcf().suptitle(t, **kwargs)


def xlabel(label, **kwargs):
    return gca().set_xlabel(label, **kwargs)


def ylabel(label, **kwargs):
    return gca().set_ylabel(label, **kwargs)


def xlim(*args, **kwargs):
    ax = gca()
    if not args and not kwargs:
        return ax.get_xlim()
    return ax.set_xlim(*args, **kwargs)


def ylim(*args, **kwargs):
    ax = gca()
    if not args and not kwargs:
        return ax.get_ylim()
    return ax.set_ylim(*args, **kwargs)


def xscale(value, **kwargs):
    return gca().set_xscale(value, **kwargs)


def yscale(value, **kwargs):
    return gca().set_yscale(value, **kwargs)


def _pyplot_ticks(axis, ticks, labels, kwargs):
    ax = gca()
    a = ax.xaxis if axis == 'x' else ax.yaxis
    if ticks is not None or labels is not None or kwargs:
        ax._set_ticks(a, ticks, labels, kwargs, axis + 'ticks')
    pairs = ax._ticks(a, ax._limits(a), 300 if axis == 'x' else 200)
    return [p for p, _ in pairs], [l for _, l in pairs]


def xticks(ticks=None, labels=None, **kwargs):
    return _pyplot_ticks('x', ticks, labels, kwargs)


def yticks(ticks=None, labels=None, **kwargs):
    return _pyplot_ticks('y', ticks, labels, kwargs)


def grid(visible=None, which='major', axis='both', **kwargs):
    return gca().grid(visible, which=which, axis=axis, **kwargs)


def tick_params(axis='both', **kwargs):
    return gca().tick_params(axis, **kwargs)


def tight_layout(*args, **kwargs):
    # Layout is always tight.
    return None


def cla():
    gca().clear()


def clf():
    gcf().clear()


def savefig(*args, **kwargs):
    raise _savefig()


def __getattr__(name):
    raise _refuse(name)
