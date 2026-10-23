# The Python side of the engine: the error values formulas see, and the few
# functions that are part of the language of errors (DESIGN.md, "Errors inside
# Python"). Run once when the engine starts. It is trusted code, so it may use
# double-underscore names that formulas may not.

from _cliotree import Node, jsonpath as _jsonpath


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
