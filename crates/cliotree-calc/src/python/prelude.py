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
    """The `ctx` of a procedure: which form and which button (or, for a write
    procedure, which field) were used, the index of the item of the repeater
    the control is in (None outside one; the innermost, when they nest), and
    the tabs of the workbook by name, which it may read and write."""

    def __init__(self, form, button, tabs, index=None):
        self.form = form
        self.button = button
        self.index = index
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


def raw(node):
    """The bytes of a node, as its tab holds them: `RAW`."""
    return _raw_access(node)


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

    def get(self, path='', params=None, headers=None):
        """An HTTP source: `GET` of `path` below its base URL. A formula may
        use it; its result refreshes when the formula runs again (F9)."""
        return self._http('GET', path, params, headers, None, None)

    def post(self, path='', json=None, data=None, params=None, headers=None):
        """`POST` to an HTTP source that allows writes, from a button
        procedure or the Immediate window. `json` is sent as JSON, `data` as
        text, or as a form when it is a dict."""
        return self._http('POST', path, params, headers, json, data)

    def put(self, path='', json=None, data=None, params=None, headers=None):
        return self._http('PUT', path, params, headers, json, data)

    def patch(self, path='', json=None, data=None, params=None, headers=None):
        return self._http('PATCH', path, params, headers, json, data)

    def delete(self, path='', params=None, headers=None):
        return self._http('DELETE', path, params, headers, None, None)

    def query(self, sql, params=None):
        """A database source: the rows of a statement that reads, as a list
        of dicts. `params` is a list (for `?` in SQLite, MariaDB and ODBC,
        `$1`, `$2` in PostgreSQL) or a dict (for `:name`, not in PostgreSQL
        or ODBC); the values are never put into the SQL text. It runs in a
        read-only transaction (for ODBC, as far as the driver allows). A
        formula may use it; its result
        refreshes when the formula runs again (F9)."""
        found = _source_call('query', self._sql(sql, params))
        names = found['columns']
        kinds = [_sql_kind(t) for t in found['types']]
        return [
            {name: _from_sql(value, kind) for name, value, kind in zip(names, row, kinds)}
            for row in found['rows']
        ]

    def execute(self, sql, params=None):
        """A database source that allows writes: runs a statement that
        changes data and gives the number of rows it changed. Only in a button
        procedure or the Immediate window."""
        return _source_call('execute', self._sql(sql, params))

    # A Beancount journal (DESIGN.md, "Beancount journals"). Amounts are
    # exact `Decimal`s and dates are `date`s.

    def transactions(self, account=None, since=None, until=None, payee=None,
                     tag=None, link=None):
        """A Beancount journal: its transactions in date order, as dicts with
        date, flag, payee, narration, tags, links, meta, file, line, padding
        and postings (each with account, amount, currency, cost,
        cost_currency, cost_date, price, price_currency, meta and auto).
        `account` includes its sub-accounts; `since` and `until` are dates,
        both included. A formula may use it; it runs again when the journal
        changes."""
        return self._journal('transactions', {
            'account': account, 'since': _day(since), 'until': _day(until),
            'payee': payee, 'tag': tag, 'link': link,
        })

    def balance(self, account, until=None):
        """A Beancount journal: what `account` and its sub-accounts hold, up
        to and including the date `until`, as a dict of currency to
        `Decimal`."""
        return self._journal('balance', {'account': account, 'until': _day(until)})

    def accounts(self):
        """A Beancount journal: its accounts, as dicts with account, open,
        close, currencies and booking."""
        return self._journal('accounts', {})

    def prices(self, currency=None):
        """A Beancount journal: its price directives, as dicts with date,
        currency, amount and quote."""
        return self._journal('prices', {'currency': currency})

    def errors(self):
        """A Beancount journal: what is wrong with it (postings that do not
        balance, accounts not open, balance assertions that fail, plugins
        that are not run), each with its file and line."""
        return self._journal('errors', {})

    def options(self):
        """A Beancount journal: its options, as a dict."""
        return self._journal('options', {})

    def add(self, date, narration, postings, payee=None, flag='*', tags=(),
            links=(), meta=None, kind='original'):
        """A Beancount journal that allows writes: appends one transaction
        to the end of its main file, from a button procedure or the
        Immediate window. Each posting is a tuple (account, amount, currency)
        or a dict with account, amount, currency, and optionally cost,
        cost_currency, price and price_currency; one may leave out its
        amount. Nothing is written unless the postings balance and the
        accounts are open. Gives the file, the line and the text added.

        Nothing ClioTree wrote is ever edited or deleted: `kind` is
        'original', 'adjustment' or 'reversal', and the narration ends with
        [ClioTree generated <kind> transaction]. `reversal()` and
        `adjustment()` work out the postings of a correction."""
        items = []
        for p in PLAIN(postings):
            if isinstance(p, (list, tuple)):
                p = dict(zip(('account', 'amount', 'currency'), p))
            if not isinstance(p, dict):
                raise TypeError('a posting is a tuple (account, amount, currency) or a dict')
            items.append({k: _to_journal(v) for k, v in p.items()})
        return self._journal('add', {
            'date': _day(date), 'narration': narration, 'payee': payee,
            'flag': flag, 'tags': list(PLAIN(tags)), 'links': list(PLAIN(links)),
            'meta': {str(k): _to_journal(v) for k, v in (PLAIN(meta) or {}).items()},
            'postings': items, 'kind': kind,
        })

    def reversal(self, transaction):
        """The postings that undo `transaction` (a dict from
        `transactions()`), for `add(..., kind='reversal')`. A formula may use
        it: it only works them out."""
        return [
            {'account': p['account'], 'amount': -p['amount'], 'currency': p['currency']}
            for p in transaction['postings'] if p['amount'] is not None
        ]

    def adjustment(self, existing, wanted):
        """The postings that take what `existing` (transactions from
        `transactions()`, such as an original and the adjustments made to it
        since) adds up to, account by account, to what `wanted` says
        (postings as for `add()`), for `add(..., kind='adjustment')`. Empty
        when nothing needs adjusting. A formula may use it: it only works
        them out."""
        import decimal
        totals = {}
        for t in PLAIN(existing):
            for p in t['postings']:
                if p['amount'] is not None:
                    key = (p['account'], p['currency'])
                    totals[key] = totals.get(key, decimal.Decimal(0)) - p['amount']
        for p in PLAIN(wanted):
            if isinstance(p, (list, tuple)):
                p = dict(zip(('account', 'amount', 'currency'), p))
            key = (p['account'], p['currency'])
            totals[key] = totals.get(key, decimal.Decimal(0)) + decimal.Decimal(str(p['amount']))
        return [
            {'account': account, 'amount': amount, 'currency': currency}
            for (account, currency), amount in totals.items() if amount != 0
        ]

    def _journal(self, method, args):
        return _from_journal(_source_call(
            'journal', {'name': self.name, 'method': method, 'args': args}))

    def _sql(self, sql, params):
        if not isinstance(sql, str):
            raise TypeError('the SQL of a query is text')
        params = PLAIN(params)
        if params is None:
            values = None
        elif isinstance(params, dict):
            values = {str(k): _to_sql(v) for k, v in params.items()}
        elif isinstance(params, (list, tuple)):
            values = [_to_sql(v) for v in params]
        else:
            raise TypeError(
                'the parameters of a query are a list (for ?) or a dict (for :name)')
        return {'name': self.name, 'sql': sql, 'params': values}

    def _http(self, method, path, params, headers, json_body, data):
        if not isinstance(path, str):
            raise TypeError('the path of a request is text, below the base URL')
        payload = {
            'name': self.name,
            'method': method,
            'path': path,
            'params': _pairs(params),
            'headers': _pairs(headers),
        }
        if json_body is not None:
            payload['body'] = _json.dumps(PLAIN(json_body))
            if not any(k.lower() == 'content-type' for k, _ in payload['headers']):
                payload['headers'].append(['Content-Type', 'application/json'])
        elif isinstance(data, dict):
            payload['form'] = _pairs(data)
        elif data is not None:
            payload['body'] = str(data)
        what = '%s: %s %s' % (self.name, method, path or '(the base URL)')
        return Response(_source_call('http', payload), what)


def _day(value):
    """A date for a journal, as ISO text: a `date` or text such as
    '2026-01-31'; None stays None."""
    value = PLAIN(value)
    if value is None or isinstance(value, str):
        return value
    import datetime
    if isinstance(value, datetime.datetime):
        return value.date().isoformat()
    if isinstance(value, datetime.date):
        return value.isoformat()
    raise TypeError('%r is not a date' % (value,))


def _to_journal(value):
    """A value for a journal: numbers as exact decimal text, dates as ISO
    text."""
    value = PLAIN(value)
    import decimal
    import datetime
    if isinstance(value, bool) or value is None or isinstance(value, str):
        return value
    if isinstance(value, (int, decimal.Decimal)):
        return {'$dec': str(value)}
    if isinstance(value, float):
        # The shortest text that reads back as the same float: 0.1, not
        # 0.1000000000000000055511151231257827.
        return {'$dec': repr(value)}
    if isinstance(value, datetime.date):
        return {'$date': _day(value)}
    raise TypeError('%s cannot go into a journal' % type(value).__name__)


def _from_journal(value):
    """What a journal gives: `{"$dec": ...}` as `Decimal` and `{"$date": ...}`
    as `date`, all the way down."""
    if isinstance(value, list):
        return [_from_journal(v) for v in value]
    if isinstance(value, dict):
        if len(value) == 1 and '$dec' in value:
            import decimal
            return decimal.Decimal(value['$dec'])
        if len(value) == 1 and '$date' in value:
            import datetime
            return datetime.date.fromisoformat(value['$date'])
        return {k: _from_journal(v) for k, v in value.items()}
    return value


def _to_sql(value):
    """A parameter as the native side takes it: JSON, with dates as ISO text
    (as SQLite keeps them) and bytes as hex."""
    value = PLAIN(value)
    if value is None or isinstance(value, (bool, int, float, str)):
        return value
    if isinstance(value, (bytes, bytearray)):
        return {'$bytes': bytes(value).hex()}
    import datetime
    if isinstance(value, datetime.datetime):
        return value.isoformat(' ')
    if isinstance(value, datetime.date):
        return value.isoformat()
    raise TypeError('%s cannot be passed to SQL: use a number, text, a date, '
                    'bytes or None' % type(value).__name__)


def _sql_kind(declared):
    """What a column's declared type says its text holds: 'datetime', 'date',
    'json', or None."""
    declared = (declared or '').upper()
    if declared == 'JSON':
        return 'json'
    if 'DATETIME' in declared or 'TIMESTAMP' in declared:
        return 'datetime'
    if 'DATE' in declared:
        return 'date'
    return None


def _from_sql(value, kind):
    if isinstance(value, dict):
        return bytes.fromhex(value['$bytes'])
    if kind is None or not isinstance(value, str):
        return value
    if kind == 'json':
        try:
            return _json.loads(value)
        except ValueError:
            return value
    import datetime
    try:
        if kind == 'date':
            return datetime.date.fromisoformat(value)
        return datetime.datetime.fromisoformat(value)
    except ValueError:
        # Text that is not a date stays text, as SQLite let it be stored.
        return value


def _text(value):
    if value is True:
        return 'true'
    if value is False:
        return 'false'
    return str(PLAIN(value))


def _pairs(mapping):
    """A dict (or a list of pairs) as a list of [name, text] pairs; a list as
    a value repeats the name."""
    if mapping is None:
        return []
    items = mapping.items() if hasattr(mapping, 'items') else mapping
    pairs = []
    for name, value in items:
        if isinstance(value, (list, tuple)):
            pairs.extend([str(name), _text(v)] for v in value)
        elif value is not None:
            pairs.append([str(name), _text(value)])
    return pairs


class Response:
    """What an HTTP source answered: `status`, `reason`, `headers` (a dict
    with lower-case names), `text`, and `json()`. A status of 400 or more is
    not a response but a `SourceError`, which is `#SOURCE!` in a formula."""

    def __init__(self, reply, what=''):
        self._what = what
        self.status = reply['status']
        self.reason = reply['reason']
        self.headers = reply['headers']
        self.text = reply['text']

    def json(self):
        try:
            return _json.loads(self.text)
        except ValueError:
            pass
        # Say what came back, so that a page where JSON was expected (a base
        # URL that is the site rather than its API) is plain to see.
        kind = self.headers.get('content-type', 'no content type')
        start = ' '.join(self.text[:80].split())
        if len(self.text) > 80:
            start += '...'
        shown = ': ' + start if start else ', and it is empty'
        raise SourceError('%s answered %s %s with %s, not JSON%s' % (
            self._what, self.status, self.reason, kind, shown))

    def __repr__(self):
        return '<Response %s %s>' % (self.status, self.reason)


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


class Record(dict):
    """An item of a repeater whose items an expression gave, as the
    expressions of the controls inside it see it: its fields can be read as
    `order['qty']` and as `order.qty`."""

    __slots__ = ()

    def __getattr__(self, name):
        try:
            return self[name]
        except KeyError:
            raise AttributeError(name) from None


# Disconnect from Data Sources writes what a source gave as a literal in the
# formula. Dates, times and decimals have no literal of their own in Python,
# so the literal calls these.

def _date(text):
    import datetime
    return datetime.date.fromisoformat(text)


def _datetime(text):
    import datetime
    return datetime.datetime.fromisoformat(text)


def _decimal(text):
    import decimal
    return decimal.Decimal(text)


def _record(fields):
    return Record(fields)


def _as_literal(value):
    """`value` as Python text that gives it back: None, booleans, numbers,
    text, bytes, dates, times, decimals, and lists, tuples, dicts and records
    of those. Anything else (an HTTP response, an error) has none."""
    import datetime
    import decimal

    def lit(v):
        if v is None or v is True or v is False:
            return repr(v)
        if isinstance(v, ErrorValue):
            raise TypeError('it gave an error value')
        if isinstance(v, Node):
            return lit(PLAIN(v))
        if isinstance(v, int):
            return repr(v)
        if isinstance(v, float):
            if v != v:
                return "float('nan')"
            if v == float('inf'):
                return "float('inf')"
            if v == float('-inf'):
                return "float('-inf')"
            return repr(v)
        if isinstance(v, (str, bytes)):
            return repr(v)
        if isinstance(v, datetime.datetime):
            return '_datetime(%r)' % v.isoformat()
        if isinstance(v, datetime.date):
            return '_date(%r)' % v.isoformat()
        if isinstance(v, decimal.Decimal):
            return '_decimal(%r)' % str(v)
        if isinstance(v, Record):
            return '_record(' + lit(dict(v)) + ')'
        if isinstance(v, dict):
            return '{' + ', '.join(lit(k) + ': ' + lit(x) for k, x in v.items()) + '}'
        if isinstance(v, list):
            return '[' + ', '.join(lit(x) for x in v) + ']'
        if isinstance(v, tuple):
            if len(v) == 1:
                return '(' + lit(v[0]) + ',)'
            return '(' + ', '.join(lit(x) for x in v) + ')'
        raise TypeError('a %s has no literal' % type(v).__name__)

    return lit(value)
