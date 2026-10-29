# The Python side of the engine: the error values formulas see, and the few
# functions that are part of the language of errors (DESIGN.md, "Errors inside
# Python"). Run once when the engine starts. It is trusted code, so it may use
# double-underscore names that formulas may not.

import json as _json

from _cliotree import Node, jsonpath as _jsonpath, xmlpath as _xmlpath


class FormulaTimeout(BaseException):
    """Raised inside a formula that has run out of time. It is not an
    `Exception`, so `except Exception` and `IFERROR` do not catch it."""


class ErrorValue:
    """What a failed node is, as a formula reads it.

    Moving through it (attributes, indexing, calling) gives the same error, so
    that `Orders.items[0].price` ends in an error and does not fail half way.
    Using it raises `PropagatedError`: arithmetic, comparison, truth, iteration,
    length, text, hashing and conversion to a number.
    """

    def __init__(self, kind, message, path, exception=None):
        self.kind = kind
        self.message = message
        self.path = path
        self.exception = exception

    def __getattr__(self, name):
        if name.startswith('__'):
            raise AttributeError(name)
        return self

    def __getitem__(self, key):
        return self

    def __call__(self, *args, **kwargs):
        return self

    def _use(self, *args, **kwargs):
        raise PropagatedError(self)

    __add__ = __radd__ = __sub__ = __rsub__ = __mul__ = __rmul__ = _use
    __truediv__ = __rtruediv__ = __floordiv__ = __rfloordiv__ = _use
    __mod__ = __rmod__ = __pow__ = __rpow__ = __neg__ = __pos__ = _use
    __abs__ = __matmul__ = __rmatmul__ = _use
    __eq__ = __ne__ = __lt__ = __le__ = __gt__ = __ge__ = _use
    __bool__ = __iter__ = __len__ = __contains__ = _use
    __str__ = __format__ = __hash__ = _use
    __int__ = __float__ = __index__ = __round__ = _use

    def __repr__(self):
        return '<ErrorValue %s %s>' % (self.kind, self.path)


class PropagatedError(Exception):
    """Raised when an `ErrorValue` is used. The exception that caused the
    original error is its `__cause__`, and its kind is the original kind."""

    def __init__(self, error):
        Exception.__init__(
            self,
            'the value at %s is an error: %s %s' % (error.path, error.kind, error.message),
        )
        self.error = error
        self.__cause__ = error.exception


class ProcedureContext:
    """The `ctx` of a button procedure: which form and which button were used,
    and the tabs of the workbook by name, which it may read and write."""

    def __init__(self, form, button, tabs):
        self.form = form
        self.button = button
        self._tabs = tabs

    def tab(self, name):
        try:
            return self._tabs[name]
        except KeyError:
            raise KeyError("there is no tab called " + repr(name)) from None


def ISERROR(value):
    """True if `value` is an error, or, if it is a function, if calling it
    raises or returns an error."""
    if isinstance(value, ErrorValue):
        return True
    if callable(value):
        try:
            result = value()
        except Exception:
            return True
        return isinstance(result, ErrorValue)
    return False


def IFERROR(value, fallback):
    """`value`, or `fallback` if `value` is an error. A function is called
    inside a `try`, so any exception, including `ZeroDivisionError`, gives the
    fallback."""
    if isinstance(value, ErrorValue):
        return fallback
    if callable(value):
        try:
            result = value()
        except Exception:
            return fallback
        return fallback if isinstance(result, ErrorValue) else result
    return value


def VALID(iterable):
    """The elements of `iterable` that are not errors, in order."""
    for element in iterable:
        if not isinstance(element, ErrorValue):
            yield element


def PLAIN(value):
    """A copy of a node as plain lists, dicts and values, reading all of it."""
    if isinstance(value, Node):
        return value.__plain__()
    if isinstance(value, (list, tuple)):
        return [PLAIN(item) for item in value]
    if isinstance(value, dict):
        return {key: PLAIN(item) for key, item in value.items()}
    return value


def JSONPATH(query, node):
    """The nodes of `node` that match a JSONPath query, as a list."""
    return _jsonpath(query, node)


def XMLPATH(query, node):
    """The nodes of `node` that match an XPath query, as a list."""
    return _xmlpath(query, node)


_range = range


def range(*args):
    result = _range(*args)
    try:
        length = len(result)
    except OverflowError:
        length = _max_range + 1
    if length > _max_range:
        raise ValueError(
            'range is longer than the limit of %d set by the Maximum range length option'
            % _max_range
        )
    return result


# Data sources (DESIGN.md, "Data sources"). The definitions belong to the user
# and the machine; a workbook names a source and this computer's definition
# says where it goes. `_source_access` is the native side (python/sources.rs).

class SourceError(Exception):
    """A data source could not be used. A formula that raises it is
    `#SOURCE!`."""


def _source_call(op, payload):
    reply = _json.loads(_source_access(op, _json.dumps(payload)))
    if 'error' in reply:
        raise SourceError(reply['error'])
    return reply['ok']


class Source:
    """A data source defined on this computer, as `source(name)` gives it: its
    name, its kind, whether it may be written to, and where it goes. Never its
    password, token or key."""

    def __init__(self, info):
        self.name = info['name']
        self.kind = info['kind']
        self.writes = info['writes']
        self.target = info['target']

    def __repr__(self):
        return '<Source %s: %s>' % (self.name, self.target)


def source(name):
    """The data source called `name` on this computer. One that is not
    defined here raises `SourceError`, which is `#SOURCE!` in a formula."""
    if not isinstance(name, str):
        raise TypeError('source() takes the name of a data source, as text')
    return Source(_source_call('info', name))


class SourceCheck:
    """What `sources.check(name)` found: `exists`, `ok` and `message`. It is
    true when the source is defined and the test passed."""

    def __init__(self, exists, ok, message):
        self.exists = exists
        self.ok = ok
        self.message = message

    def __bool__(self):
        return self.ok

    def __repr__(self):
        return '<SourceCheck exists=%r ok=%r: %s>' % (self.exists, self.ok, self.message)


class _Sources:
    """`sources`: defining and checking data sources on this computer, from a
    button procedure or the Immediate window."""

    def define(self, name, kind, *, replace=False, **settings):
        """Defines (or, with `replace=True`, replaces) the data source `name`
        on this computer. `kind` is "http", "sqlite", "postgres" or "mariadb";
        the settings are those of the definition form: host, port, database,
        user, password, url, headers, auth, token, key_name, test_path, path,
        pool and writes."""
        if not isinstance(name, str) or not isinstance(kind, str):
            raise TypeError('sources.define() takes the name and the kind as text')
        _source_call('define', {
            'name': name, 'kind': kind, 'settings': settings, 'replace': bool(replace),
        })

    def check(self, name):
        """Whether the data source `name` is defined on this computer and
        passes the same test as the Test button, and why not."""
        if not isinstance(name, str):
            raise TypeError('sources.check() takes the name of a data source, as text')
        found = _source_call('check', name)
        return SourceCheck(found['exists'], found['ok'], found['message'])

    def __repr__(self):
        return '<data sources>'


sources = _Sources()
