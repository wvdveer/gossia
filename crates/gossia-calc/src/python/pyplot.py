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

    def __init__(self, name):
        self.name = name
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


class _Line:
    def __init__(self, xs, ys, style):
        self.xs = xs
        self.ys = ys
        self.style = style
        self.label = style.get('label')

    def bounds(self, axes):
        return _points_bounds(self.xs, self.ys, axes)

    def sticky(self):
        return [], []

    def draw(self, out, axes, box):
        style = self.style
        color = style['color']
        points = []
        runs = []
        for x, y in zip(self.xs, self.ys):
            p = axes._place(x, y, box)
            if p is None:
                if points:
                    runs.append(points)
                points = []
            else:
                points.append(p)
        if points:
            runs.append(points)
        width = style.get('linewidth', 1.5)
        alpha = style.get('alpha')
        opacity = ' opacity="%s"' % _fmt(alpha) if alpha is not None else ''
        dash = style.get('dash')
        if dash != 'none':
            dash_attr = ' stroke-dasharray="%s"' % ','.join(
                _fmt(float(d) * width / 1.5) for d in dash.split(',')) if dash else ''
            for run in runs:
                if len(run) < 2:
                    continue
                d = 'M' + ' L'.join('%s %s' % (_fmt(px), _fmt(py)) for px, py in run)
                out.append('<path d="%s" fill="none" stroke="%s" stroke-width="%s"'
                           ' stroke-linejoin="round" stroke-linecap="butt"%s%s/>'
                           % (d, color, _fmt(width), dash_attr, opacity))
        marker = style.get('marker')
        if marker:
            size = style.get('markersize', 6.0)
            face = style.get('markerfacecolor') or color
            for run in runs:
                for px, py in run:
                    out.append(_marker(marker, px, py, size, face, color, opacity))


def _marker(marker, x, y, size, face, edge, opacity=''):
    r = size / 2.0
    common = ' fill="%s" stroke="%s" stroke-width="1"%s' % (face, edge, opacity)
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
    width = 2.0 if marker == 'X' else 1.0
    d = ' '.join('M%s %s L%s %s' % (_fmt(x + a * r), _fmt(y + b * r), _fmt(x + c * r), _fmt(y + e * r))
                 for a, b, c, e in lines[marker])
    return '<path d="%s" fill="none" stroke="%s" stroke-width="%s"%s/>' % (d, edge, _fmt(width), opacity)


def _points_bounds(xs, ys, axes):
    px = [x for x, y in zip(xs, ys) if x is not None and y is not None
          and axes._usable(axes.xaxis, x) and axes._usable(axes.yaxis, y)]
    py = [y for x, y in zip(xs, ys) if x is not None and y is not None
          and axes._usable(axes.xaxis, x) and axes._usable(axes.yaxis, y)]
    if not px:
        return None
    return min(px), max(px), min(py), max(py)


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


class Axes:
    """A plot area of a figure, with its x and y axes."""

    def __init__(self, figure, row, col, rows, cols):
        self.figure = figure
        self.cell = (row, col, rows, cols)
        self.xaxis = _Axis('x')
        self.yaxis = _Axis('y')
        self.artists = []
        self.titles = {}
        self.grid_on = {'x': False, 'y': False}
        self.cycle = 0

    def __getattr__(self, name):
        raise _refuse('Axes.%s' % name)

    # -- what is drawn

    def _next_colour(self):
        colour = _CYCLE[self.cycle % len(_CYCLE)]
        self.cycle += 1
        return colour

    def plot(self, *args, **kwargs):
        _check(kwargs, {'color', 'c', 'linestyle', 'ls', 'linewidth', 'lw', 'label',
                        'marker', 'markersize', 'ms', 'markerfacecolor', 'mfc',
                        'alpha', 'scalex', 'scaley', 'data'}, 'plot')
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
            style = {}
            colour = kwargs.get('color', kwargs.get('c', color))
            style['color'] = _colour(colour) if colour is not None else self._next_colour()
            line = kwargs.get('linestyle', kwargs.get('ls', line))
            if line is not None and line not in _LINESTYLES:
                raise ValueError('%r is not a line style' % line)
            style['dash'] = _LINESTYLES.get(line) if line is not None else None
            style['linewidth'] = float(kwargs.get('linewidth', kwargs.get('lw', 1.5)))
            marker = kwargs.get('marker', marker)
            if marker in ('', 'None', 'none'):
                marker = None
            if marker is not None and (not isinstance(marker, str) or marker not in _MARKERS + '1234'):
                raise ValueError('gossia.pyplot does not support the marker %r' % (marker,))
            style['marker'] = marker
            style['markersize'] = float(kwargs.get('markersize', kwargs.get('ms', 6.0)))
            face = kwargs.get('markerfacecolor', kwargs.get('mfc'))
            style['markerfacecolor'] = _colour(face) if face is not None else None
            if kwargs.get('alpha') is not None:
                style['alpha'] = float(kwargs['alpha'])
            if kwargs.get('label') is not None:
                style['label'] = str(kwargs['label'])
            item = _Line(xs, ys, style)
            self.artists.append(item)
            lines.append(item)
        return lines

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
        if value not in ('linear', 'log'):
            raise ValueError("gossia.pyplot does not support the scale %r (only 'linear' and 'log')"
                             % (value,))
        if value == 'log' and axis.kind in ('date', 'category'):
            raise ValueError('an axis of dates or text cannot be logarithmic')
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
        for artist in self.artists:
            b = artist.bounds(self)
            if b is not None:
                found.append(b[:2] if axis is self.xaxis else b[2:])
            s = artist.sticky()
            sticky.extend(s[0] if axis is self.xaxis else s[1])
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
        margin = (dhi - dlo) * 0.05
        if axis.kind == 'category':
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
        figure = self.figure
        cell = self.cell
        self.__init__(figure, *cell)

    cla = clear


def _scale(axis, value, limits, start, end):
    lo, hi = limits
    if axis.inverted:
        lo, hi = hi, lo
    if axis.scale == 'log':
        value, lo, hi = _math.log10(value), _math.log10(lo), _math.log10(hi)
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

    def subplots(self, nrows=1, ncols=1, squeeze=True, **kwargs):
        _check(kwargs, set(), 'subplots')
        nrows, ncols = int(nrows), int(ncols)
        if nrows < 1 or ncols < 1:
            raise ValueError('subplots needs at least one row and one column')
        grid = [[self.add_subplot(nrows, ncols, r * ncols + c + 1) for c in range(ncols)]
                for r in range(nrows)]
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


def _layout(ax, cell):
    """The plot box of an Axes in its cell, with its ticks: three rounds,
    since the ticks depend on the box's size and the margins on the ticks."""
    x0, y0, x1, y1 = cell
    has_title = bool(ax.titles)
    title_h = max([s for _, s in ax.titles.values()] or [0]) * _LINE + 4 if has_title else 0.0
    xlabel_h = ax.xaxis.label_size * _LINE + 2 if ax.xaxis.label else 0.0
    ylabel_w = ax.yaxis.label_size * _LINE + 2 if ax.yaxis.label else 0.0
    xlim = ax._limits(ax.xaxis)
    ylim = ax._limits(ax.yaxis)
    box = (x0 + _PAD + 30, y0 + _PAD + title_h, x1 - _PAD - 10, y1 - _PAD - 24)
    for _ in range(3):
        left, top, right, bottom = box
        xt = ax._ticks(ax.xaxis, xlim, right - left)
        yt = ax._ticks(ax.yaxis, ylim, bottom - top)
        yoff = ax._offset(ax.yaxis, ylim, bottom - top)
        ywide = max([_width(l, ax.yaxis.tick_size) for _, l in yt] or [0])
        xhigh = _rotated_height([l for _, l in xt], ax.xaxis.tick_size, ax.xaxis.rotation)
        # Room for half of the last x label beyond the box, when it is not rotated.
        overhang = 0.0
        if xt and not ax.xaxis.rotation:
            last = xt[-1]
            pos = _scale(ax.xaxis, last[0], xlim, left, right) if ax._usable(ax.xaxis, last[0]) else right
            overhang = max(0.0, pos + _width(last[1], ax.xaxis.tick_size) / 2 - right)
        top_extra = ax.yaxis.tick_size * _LINE if yoff else 0.0
        box = (x0 + _PAD + ylabel_w + ywide + _TICK_LEN + _TICK_GAP,
               y0 + _PAD + max(title_h, top_extra),
               x1 - _PAD - overhang,
               y1 - _PAD - xlabel_h - xhigh - _TICK_LEN - _TICK_GAP)
    left, top, right, bottom = box
    if right - left < 10 or bottom - top < 10:
        raise ValueError('the chart is too small for its labels')
    xt = ax._ticks(ax.xaxis, xlim, right - left)
    yt = ax._ticks(ax.yaxis, ylim, bottom - top)
    return box, xlim, ylim, xt, yt


def _draw_axes(out, ax, cell, clip_id):
    box, xlim, ylim, xt, yt = _layout(ax, cell)
    ax._view = (xlim, ylim)
    left, top, right, bottom = box
    x0, y0, x1, y1 = cell
    grid = []
    ticks = []
    # Ticks and their labels.
    for pos, label in xt:
        if not ax._usable(ax.xaxis, pos):
            continue
        x = _scale(ax.xaxis, pos, xlim, left, right)
        if x < left - 0.5 or x > right + 0.5:
            continue
        ticks.append('M%s %s V%s' % (_fmt(x), _fmt(bottom), _fmt(bottom + _TICK_LEN)))
        if ax.grid_on['x']:
            grid.append('M%s %s V%s' % (_fmt(x), _fmt(top), _fmt(bottom)))
        size = ax.xaxis.tick_size
        y = bottom + _TICK_LEN + _TICK_GAP + size * _ASCENT
        if ax.xaxis.rotation:
            _text(out, x, bottom + _TICK_LEN + _TICK_GAP + size * 0.5, label, size,
                  anchor='end', rotate=-ax.xaxis.rotation, opacity=0.8)
        else:
            _text(out, x, y, label, size, opacity=0.8)
    for pos, label in yt:
        if not ax._usable(ax.yaxis, pos):
            continue
        y = _scale(ax.yaxis, pos, ylim, bottom, top)
        if y < top - 0.5 or y > bottom + 0.5:
            continue
        ticks.append('M%s %s H%s' % (_fmt(left - _TICK_LEN), _fmt(y), _fmt(left)))
        if ax.grid_on['y']:
            grid.append('M%s %s H%s' % (_fmt(left), _fmt(y), _fmt(right)))
        size = ax.yaxis.tick_size
        _text(out, left - _TICK_LEN - _TICK_GAP, y + size * (_ASCENT / 2), label, size,
              anchor='end', opacity=0.8)
    offset = ax._offset(ax.yaxis, ylim, bottom - top)
    if offset:
        _text(out, left, top - 3, offset, ax.yaxis.tick_size, anchor='start', opacity=0.8)
    xoffset = ax._offset(ax.xaxis, xlim, right - left)
    if xoffset:
        _text(out, right, bottom + _TICK_LEN + _TICK_GAP + ax.xaxis.tick_size * (_ASCENT + _LINE),
              xoffset, ax.xaxis.tick_size, anchor='end', opacity=0.8)
    if grid:
        out.insert(0, '<path d="%s" fill="none" stroke="currentColor" stroke-opacity="0.16"'
                      ' stroke-width="0.8"/>' % ' '.join(grid))
    # What is drawn, clipped to the box.
    out.append('<clipPath id="%s"><rect x="%s" y="%s" width="%s" height="%s"/></clipPath>'
               % (clip_id, _fmt(left), _fmt(top), _fmt(right - left), _fmt(bottom - top)))
    out.append('<g clip-path="url(#%s)">' % clip_id)
    for artist in ax.artists:
        artist.draw(out, ax, box)
    out.append('</g>')
    # The frame and the ticks.
    out.append('<path d="%s" fill="none" stroke="currentColor" stroke-opacity="0.8"'
               ' stroke-width="0.8"/>' % ' '.join(ticks))
    out.append('<rect x="%s" y="%s" width="%s" height="%s" fill="none" stroke="currentColor"'
               ' stroke-opacity="0.8" stroke-width="0.8"/>'
               % (_fmt(left), _fmt(top), _fmt(right - left), _fmt(bottom - top)))
    # Labels and titles.
    if ax.xaxis.label:
        size = ax.xaxis.label_size
        _text(out, (left + right) / 2, y1 - _PAD - size * (_LINE - _ASCENT), ax.xaxis.label, size)
    if ax.yaxis.label:
        size = ax.yaxis.label_size
        x = x0 + _PAD + size * _ASCENT
        _text(out, x, (top + bottom) / 2, ax.yaxis.label, size, rotate=-90)
    for loc, (title, size) in ax.titles.items():
        x = {'center': (left + right) / 2, 'left': left, 'right': right}[loc]
        anchor = {'center': 'middle', 'left': 'start', 'right': 'end'}[loc]
        _text(out, x, top - 6, title, size, anchor=anchor)


def _render(fig):
    out = []
    top = 0.0
    if fig.title:
        text, size = fig.title
        top = _PAD + size * _LINE
        _text(out, fig.width / 2, _PAD + size * _ASCENT, text, size, weight='600')
    axes = fig.axes or [Axes(fig, 0, 0, 1, 1)]
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


def subplots(nrows=1, ncols=1, *, squeeze=True, figsize=None, width=None, height=None, **kwargs):
    _check(kwargs, {'sharex', 'sharey', 'subplot_kw', 'gridspec_kw', 'layout'}, 'subplots')
    for key in ('sharex', 'sharey', 'subplot_kw', 'gridspec_kw'):
        if kwargs.get(key):
            raise TypeError('gossia.pyplot does not support the argument %s of subplots' % key)
    fig = figure(figsize=figsize, width=width, height=height)
    return fig, fig.subplots(nrows, ncols, squeeze=squeeze)


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
