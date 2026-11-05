# The Excel-named functions (DESIGN.md, "Semantics: Python first, Excel second").
#
# They are thin Python over the standard library, written for people who know
# Python: no value is turned into another kind, an empty `None` is not skipped,
# and an `ErrorValue` among the arguments raises `PropagatedError`, so a
# function never hides a bad item. Run once when the engine starts, in the
# namespace of the prelude, so it can use `ErrorValue`, `PropagatedError`,
# `Node` and `PLAIN`. It is trusted code.


from _gossia import (
    base64encode as _base64encode,
    base64decode as _base64decode,
    hexencode as _hexencode,
    hexdecode as _hexdecode,
)


class NotAvailable(Exception):
    """Raised by `NA()`. It is what a node holding `#N/A` came from."""


def _flat(args):
    """The elements of the arguments, one level deep. A list, a tuple or an
    array gives its elements; anything else, text included, is one element."""
    for arg in args:
        if isinstance(arg, ErrorValue):
            raise PropagatedError(arg)
        if isinstance(arg, (str, bytes, dict)):
            yield arg
        elif isinstance(arg, Node):
            if arg.__kind__() == 'array':
                yield from arg
            else:
                yield arg
        elif hasattr(arg, '__iter__'):
            yield from arg
        else:
            yield arg


def _field(item, name):
    """The field `name` of an object of the data, or of a dict."""
    if isinstance(item, ErrorValue):
        raise PropagatedError(item)
    return item[name]


def _frozen(value):
    """A hashable stand-in for a value, so that equal values can be found."""
    value = PLAIN(value)
    if isinstance(value, (list, tuple)):
        return tuple(_frozen(v) for v in value)
    if isinstance(value, dict):
        return tuple(sorted((k, _frozen(v)) for k, v in value.items()))
    return value


def SUM(*values):
    """The sum of the numbers in the arguments; `0` if there are none."""
    return sum(_flat(values))


def AVERAGE(*values):
    """The mean of the numbers in the arguments. With none, `ZeroDivisionError`
    (`#DIV/0!`)."""
    items = list(_flat(values))
    return sum(items) / len(items)


def MIN(*values, **options):
    """The smallest element. Numbers, text and dates all compare. `default=`
    is the result when there are no elements, as for Python's `min`."""
    return min(_flat(values), **options)


def MAX(*values, **options):
    """The largest element. `default=` is the result when there are none."""
    return max(_flat(values), **options)


def COUNT(*values):
    """How many elements are not `None`. An error among them is raised."""
    count = 0
    for item in _flat(values):
        if isinstance(item, ErrorValue):
            raise PropagatedError(item)
        if item is not None:
            count += 1
    return count


def IF(condition, then, otherwise=None):
    """`then` if `condition` is true, else `otherwise`. Both are worked out
    before the call; `a if c else b` works out only the one it needs."""
    return then if condition else otherwise


def LOOKUP(key, items, key_field, result_field=None):
    """`result_field` of the first of `items` whose `key_field` equals `key`,
    or without `result_field` the whole item. No such item raises `KeyError`
    (`#REF!`)."""
    for item in items:
        if _field(item, key_field) == key:
            if result_field is None:
                return item
            return _field(item, result_field)
    raise KeyError('no item has %s equal to %r' % (key_field, key))


def FILTER(items, condition):
    """The items for which `condition` is true, as a list. `condition` is a
    function of one item, or a list with a true or false for each item."""
    items = list(items)
    if callable(condition):
        return [item for item in items if condition(item)]
    flags = list(condition)
    if len(flags) != len(items):
        raise ValueError(
            'FILTER needs a flag for each item: %d items, %d flags' % (len(items), len(flags))
        )
    return [item for item, keep in zip(items, flags) if keep]


def SORT(items, key=None, reverse=False):
    """The items in order, as a list. `key` is a function of one item, or the
    name of a field."""
    if isinstance(key, str):
        name = key
        key = lambda item: _field(item, name)
    return sorted(items, key=key, reverse=reverse)


def UNIQUE(items):
    """The items without repeats, as a list, each kept at its first place.
    Objects and lists are equal when everything in them is."""
    seen = set()
    found = []
    for item in items:
        mark = _frozen(item)
        if mark not in seen:
            seen.add(mark)
            found.append(item)
    return found


def CONCAT(*arrays):
    """One list holding the items of each array in turn. A value that is not
    an array raises `TypeError` (`#TYPE!`)."""
    joined = []
    for array in arrays:
        if isinstance(array, ErrorValue):
            raise PropagatedError(array)
        if isinstance(array, Node):
            if array.__kind__() != 'array':
                raise TypeError('CONCAT takes arrays, not an object')
            joined.extend(array)
        elif isinstance(array, (list, tuple)):
            joined.extend(array)
        else:
            raise TypeError('CONCAT takes arrays, not %s' % type(array).__name__)
    return joined


def TEXTJOIN(delimiter, *values):
    """The text elements of the arguments joined with `delimiter`. An element
    that is not text raises `TypeError`."""
    parts = []
    for item in _flat(values):
        if isinstance(item, ErrorValue):
            raise PropagatedError(item)
        parts.append(item)
    return delimiter.join(parts)


def TEXT(value, spec=''):
    """`value` as text by a Python format spec: `TEXT(3.14159, '.2f')`."""
    return format(value, spec)


def NA():
    """Raises `#N/A`, for a value that is not available."""
    raise NotAvailable('not available')


def _not_error(value):
    if isinstance(value, ErrorValue):
        raise PropagatedError(value)
    return value


def RAW(node):
    """The bytes of a node as its tab holds them (DESIGN.md, "Raw memory"):
    a COBOL field's own bytes, the record for the root, or the node written
    as text in its tab's format."""
    return _raw_access(node)


def BASE64ENCODE(data):
    """Bytes as Base64 text."""
    return _base64encode(_not_error(data))


def BASE64DECODE(text):
    """The bytes of Base64 text."""
    return _base64decode(_not_error(text))


def HEXENCODE(data):
    """Bytes as hex digits, two upper-case digits a byte."""
    return _hexencode(_not_error(data))


def HEXDECODE(text):
    """The bytes of hex digits; white space between them is passed over."""
    return _hexdecode(_not_error(text))
