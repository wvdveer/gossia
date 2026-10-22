# Cliotree Design

A spreadsheet-like desktop application (Rust + egui, later wasm) in which each sheet is a **tree defined by a schema** (JSON Schema or XSD) rather than a 2D grid. Cells at any node can hold a **Python expression** whose result must match the node's schema type.

**Scope.** Version 1 is the desktop application. **Reports** and the **web (wasm) port** are deliberately deferred to a later version; their designs are kept in this document for planning and are marked as such (see Later versions).

## Core model

**Workbook**: named Sheets (tabs), a trust flag, and embedded Python modules. Tabs come in two kinds:
- **Tree sheets** (data sheets and schema sheets): hold a data tree bound to a schema, as described below.
- **Reports** *(later version)*: read-only visual presentations of data from other tabs (see Reports).

Forms are not tabs; see Forms.

Rules for tabs:
- Tab names must be valid Python identifiers, must not be reserved words, and must be unique ignoring case (enforced on create and rename; see Names and reserved words).
- Each tree sheet has a schema reference, a data tree, and formula overlays.

**Data tree**: one format-neutral `Value` enum serves JSON and XML:
`Null | Bool | Int(i64) | Float(f64) | Str | Date | DateTime | Array(Vec<Node>) | Object(IndexMap<String, Node>) | Error(ErrorValue)`.
`Error` exists only in memory: it is how a failed formula's node is represented, and it is never written into a data file (see Stored values).
- Each node has a stable `NodeId`.
- XML maps onto the same model: attributes become fields prefixed `@`, text becomes `#text`, repeated elements become arrays. A format adapter handles the round trip; the evaluator never sees XML or JSON.
- Data format follows the schema language: JSON Schema gives JSON data, XSD gives XML data. The two meta-schemas (JSON Schema, XSD) are built in, so schema sheets get ordinary tree editing and validation (see Schema binding).

**Notes and structure fidelity.** Round-tripping is lossy only for incidental formatting. What the user wrote deliberately is preserved.
- **Field order.** Object fields keep the order of the schema's declared properties (JSON) or sequence (XML), and array items keep their order. Export and save write them in that order. A schema's declared order is itself user-authored and preserved.
- **Notes (comments).** Any node can carry a **note**, a free-text comment like an Excel cell comment, shown and edited in the UI. Notes are part of the node in memory, so they move with it when items are inserted, deleted, or sorted (unlike formulas, which belong to a location). When saved they are written keyed by path, in the overlay for JSON tabs.
  - On XML tabs, notes are stored as real **XML comments** in `data.xml`, so they travel with the file. A comment immediately before an element or attribute becomes that node's note. Comments elsewhere (before the root, after the root, at the end of an element) become notes on the root or on the enclosing element, tagged with their position, and are written back in the same place.
  - JSON has no comments, so JSON tabs keep their notes only in the overlay. A standalone JSON export drops them.
- **XML details preserved:** the XML declaration (version, encoding, standalone), namespace prefix and declarations, attribute order, element order.
- **Not preserved (accepted):** indentation and other insignificant whitespace (regenerated on write), CDATA sections (become ordinary text), entity and character-reference spelling, attribute quote style, empty-element syntax (`<a/>` vs `<a></a>`), processing instructions, and DOCTYPE declarations (dropped with a warning; DTD entities are never expanded and external entities are never fetched). Text content inside an element is preserved exactly, including its whitespace.
- **JSON details not preserved:** number spelling (`1e3` becomes `1000`, `1.0` stays a float), and indentation. Integers beyond the signed 64-bit range import as floating-point numbers, with a warning.

**Schema model**: JSON Schema and XSD are lowered once, at load, into a single internal `TypeSpec`:
- Kinds: `String | Integer | Number | Bool | Array(items, min/max) | Object(fields, required) | Map(value) | Enum`.
- `Map` is a string-keyed dictionary with a uniform value type. It comes only from JSON Schema (`additionalProperties` with a schema) and has no XSD counterpart.
- Named types may be referenced recursively (types are held by id, not inlined). Data is finite, so recursion terminates.
- Constraints: pattern, range, length.
- `OneOf` is reserved for a later version and is not produced in v1 (both subsets exclude unions).
- A source map points each `TypeSpec` back to its schema location for error messages.

**Schema binding**: every tree sheet declares its schema in exactly one of two ways:
- **Built-in meta-schema**: the sheet is itself a schema document. The application bundles a JSON Schema for JSON Schema and an XML Schema for XML Schema, each describing exactly the supported subset of its language (see below). Choosing one fixes the sheet's format (JSON or XML), and the sheet is edited, validated, and given formulas like any other tree.
- **Sheet reference**: the sheet names another workbook sheet that contains its schema. That sheet must itself be a schema sheet (bound to a built-in meta-schema). The referenced sheet's schema language determines the data format: JSON Schema gives JSON data, XSD gives XML data.

Rules:
- The built-ins are bundled in the binary, read-only, and are not workbook sheets. They are referenced by identifier (e.g. `builtin:jsonschema`, `builtin:xsd`), never by sheet name.
- A sheet reference must resolve to a schema sheet in the same workbook. A missing sheet, a non-schema sheet, or a cycle in the chain is a load-time error shown on the sheet.
- When a schema sheet is edited, every sheet bound to it is re-lowered to `TypeSpec` and revalidated, and dependent formula results are rechecked.
- A schema sheet must be valid against its meta-schema, and also lowerable to `TypeSpec` (so it may use only the supported subset of its language).
- Neither bundled meta-schema is the official one (the W3C `XMLSchema.xsd` or the JSON Schema 2020-12 meta-schema). Each is our own schema that describes exactly the supported v1 subset, written using only that subset, and lowered through the normal path like any other schema. The two are kept parallel: the same concepts, the same restrictions, and the same exclusions. Widening a subset means updating its meta-schema in the same change.
- Meta-schemas without unions can only approximate the language. Anything the structure can't express is caught by lowering, which reports precise errors (e.g. an `enum` with a bad value) and is the authoritative check.

### Schema subsets (v1)

Both languages are reduced to the same core: closed record types, arrays, and simple types with a few restrictions. The features line up as follows.

| Concept | JSON Schema (2020-12) | XSD |
|---|---|---|
| Record with ordered fields | `type: object` with `properties` (declared order kept) | `xs:complexType` with `xs:sequence` of `xs:element` |
| Optional / required field | name absent from / present in `required` | `minOccurs` 0 / 1 |
| Array | `type: array`, `items`, `minItems`, `maxItems` | `maxOccurs` > 1 (or `unbounded`) on an element |
| Attributes | not applicable | `xs:attribute`; text via `xs:simpleContent` |
| Reusable types, recursion | `$defs` plus local `$ref: "#/$defs/Name"` | named `xs:complexType` / `xs:simpleType`, `type=` references, `xs:element ref` |
| String / boolean | `string`, `boolean` | `string`, `boolean` |
| Numbers | `integer`, `number` | `integer`, `decimal`, `double` |
| Dates | `string` with `format: date` / `date-time` (a string constraint only; the node stays a string) | `date`, `dateTime` (typed nodes holding dates) |
| Enumeration | `enum` | `xs:enumeration` |
| Pattern | `pattern` | `xs:pattern` |
| Numeric range (inclusive) | `minimum`, `maximum` | `minInclusive`, `maxInclusive` |
| Length | `minLength`, `maxLength` | `minLength`, `maxLength` |
| Documentation | `title`, `description` (annotation only) | `xs:annotation` / `xs:documentation` (annotation only) |
| Map (dictionary) | `additionalProperties: <schema>`, instead of `properties` | none |

Objects are **closed**: no properties beyond those declared. The one deviation is `Map`, which allows any string key with one value type, and is the only way for an object to have undeclared keys. This is needed because the JSON Schema meta-schema itself contains such maps (`properties` and `$defs`). A schema object uses `properties` or a schema-valued `additionalProperties`, not both.

A schema document has one root: the top-level schema for JSON Schema, and the single global `xs:element` for XSD (other global declarations must be types).

Excluded from both, with the same intent: unions and alternatives (`oneOf` / `anyOf` / `allOf` / `not`, `xs:choice`, `union`), conditional schemas (`if` / `then` / `else`), nullable / `null` (`type` arrays, `nillable`), open content (`patternProperties`, `unevaluated*`, `xs:any`), type extension and derivation, remote or non-local references (`$ref` to a URL or `$anchor`, `xs:import` / `xs:include`), identity constraints (`uniqueItems`, `key` / `keyref`), exclusive ranges, `multipleOf`, `const`, tuple arrays (`prefixItems`), `contains`, `dependent*`, mixed content, multiple namespaces, `list`.

An unsupported construct or keyword fails at schema load with an error naming it and its location. Nothing is silently ignored. Unknown annotation keys are also an error, so typos aren't hidden.

Numeric enumerations are not in v1: `enum` is limited to string-typed nodes, because the meta-schemas have no way to say "any scalar" without a union. Both languages share this limit.

### Notes on the XSD side

- `maxOccurs="unbounded"` is a union in the official XSD, so the meta-schema types it as a string with a `pattern` of `[0-9]+|unbounded`.
- Other numeric and string built-in types (`int`, `long`, `float`, `token`, ...) alias to the listed ones.
- Type references resolve within one schema document and one target namespace.

### Likely first additions

Unions (`xs:choice`, `oneOf` / `anyOf`) are the most probable first widening. They lower to `OneOf`, which is why that kind is reserved, and nullable types and numeric enums come next. Each widens both languages together.

## Formulas

A formula is a Python expression string attached to a node **location** (its path), with a cached result. Formulas are stored beside the data (a parallel `formulas` map keyed by path), not inside it, so the data stays valid JSON / XML.

**Entering formulas.** Formulas are entered as in Excel.
- A **formula bar** sits above the tree grid. For the selected node it shows the node's path (for example `Orders.items[2].price`) and its content: the formula text if it has one, otherwise the literal value. Cells can also be edited in place (F2 or double-click), and the bar and the cell edit the same text.
- Text that **starts with `=`** is a formula, and what follows is a single Python expression: `=SUM(Orders.items.price)`. Anything else is a literal, parsed to the node's type.
- To enter a literal string that begins with `=`, start it with an apostrophe: `'=total` is stored as `=total`. A single leading apostrophe on entry is removed, so a literal that really starts with an apostrophe is typed with two. The apostrophe rule applies only to text-typed nodes.
- The expression is a Python *expression*, not statements. It may span lines (Shift+Enter in the bar) when the line breaks fall inside brackets, and may use lambdas and comprehensions. Surrounding whitespace is ignored.
- Enter commits and Esc cancels. A commit is one undoable entry. Replacing a formula with a literal removes the formula. Clearing the text removes both, leaving an absent optional node or an empty flagged required one.
- Committing queues a recalculation. The cell shows its last value (or empty) with the pending marker until the result arrives (see Evaluation and scheduling).
- Pasting a formula pastes its text verbatim, with no reference adjustment, as references are literal.
- On a node inside a computed subtree the bar is read-only and shows which formula and location computes it.
- Report expressions, form button conditions, and similar designer fields take a bare Python expression and do not use the `=` prefix.

**Syntax errors.** A formula whose text does not parse is **not rejected**. It is kept, and the node holds the error value `#SYNTAX!`, whose message gives Python's description and the line and column. The text stays in the overlay so the author can correct it. As for other errors, dependents see the error (see Error values), and the `ErrorValue`'s `.exception` is the `SyntaxError`.
- Parsing only reads the text and never runs it, so the syntax check also happens in an untrusted workbook.
- A formula rejected by the double-underscore attribute check (see Python engine) is reported the same way, as `#SYNTAX!` with a message saying that double-underscore access is not allowed.
- While the author types, the bar shows a non-blocking syntax hint. It never prevents committing.
- There is no autocomplete in v1.

**Stored values.** A formula's result is written into the data tree at its node and saved with the sheet, so the data file is complete and valid JSON / XML on its own. Other tools can read it without running any Python. The formula text and its status live only in the overlay.
- On save, the tree holds the latest computed values and the overlay holds the formulas. Readers that ignore the overlay see plain data.
- On open, a **trusted** workbook is fully recalculated, so volatile functions (`NOW`, `RAND`) take fresh values. An **untrusted** workbook shows the stored values and runs nothing (see Trust model).
- Computed array and object results are materialised as ordinary children in the tree. They are marked computed by the formula at the subtree root, and the UI treats them as read-only. If a file is edited externally, the next recalculation overwrites computed values.
- **A failed formula puts an error value in the node.** If the formula raises, or returns something that doesn't match the node's schema type, the node's value becomes an error value (`#TYPE!`, `#DIV/0!`, ...), as in Excel. The previous value is discarded, so there is never a stale value to mislead. The error's kind, message, and traceback are kept in the overlay, and dependents see the error (see Error values).
- **Errors in saved files.** JSON and XML cannot hold error values, so the data file is written with an errored node **absent**, and the overlay records the error. On open of a trusted workbook the formula is recalculated anyway; on an untrusted one the overlay restores the error display without running anything. An errored required node therefore makes the plain data file schema-invalid, which is accepted, since the file is in an error state.
- **Absent nodes.** A formula may be attached to an optional node that is currently absent; a successful result creates the node. A result of `None` means "no value": the optional node is left absent, and for a required node it is a `#TYPE!` error. There is no null in v1 schemas.
- A formula can only be entered on a node the schema defines (see Cells exist only where the schema says), and not on a node whose ancestor already carries a formula.

**Cells exist only where the schema says.** A node can hold a value or a formula only if the schema defines it: a leaf, or an array or object node at any level. The schema, not the data, decides where cells are.
- Every schema-defined node has a cell, whether or not it currently has data. An absent optional node shows an empty cell that can be filled with a literal or a formula; array items and map entries get cells as they are added.
- Content in a loaded data file that the schema does not define (an unknown key, element, or attribute) gets **no cell**. It is preserved so the file round-trips, shown greyed and read-only in an "unrecognised" group on its parent, can be deleted, and is listed in the issues panel. It cannot carry a formula or be referenced by one.

**Invalid data.** The application never refuses or discards data because it doesn't validate.
- A literal in a schema-defined node that violates its type or constraints (from a loaded file, or because the schema was edited) is kept, drawn with a red outline, and listed in the issues panel. Reading it from a formula returns it as-is. The UI editors prevent entering such a literal in the first place.
- A missing required node shows as an empty flagged cell.
- A **formula** is stricter: its result must match the schema, or the node holds an error value instead (see Stored values). A formula never writes an invalid value.

**References are literal.** A path written in a formula means exactly that path, always. `Orders.items[2]` is the third item of `Orders.items` under any structural edit (insert, delete, sort); nothing rewrites formula text, and there is no Excel-style reference adjustment. Likewise a formula belongs to its location, not to the item currently there. Authors who want references that follow the data's structure compute them:
- by data, e.g. `Orders.items[Data.relevantOrder]`;
- by query, e.g. `JSONPATH("$.items[?(@.id == 7)]", Orders)` or `XMLPATH(...)`;
- by expression, e.g. a comprehension with a condition.

**Renames do not rewrite formulas.** The same principle applies to names. Renaming a tab or a schema field (or removing it) leaves every formula, form button, and report expression that mentions it untouched. Those expressions then fail at their next evaluation: a missing tab or module gives `#NAME!`, and a missing field gives `#REF!`. The error message names the missing identifier so the author can find and fix it. The application may offer a find-and-replace across formulas as a convenience, but never does so implicitly.

An index that falls outside the array after an edit yields `#REF!` like any other out-of-range access. `NodeId`s exist for UI identity (selection, expansion state, undo) and are not used by formulas.

**Name resolution**
- Each sheet name is injected as a global proxy object, e.g. `Orders.items[2].price`.
- Proxies resolve lazily via `__getattr__` / `__getitem__`; every access records a dependency edge, so no static parsing is needed.
- Relative addressing: `this`, `parent`, `root`.
- `JSONPATH("$.items[*].price", sheet)` and `XMLPATH(...)` cover queries that don't fit member syntax. They return live proxies or lists of them; each match is a dependency.

**Dependency graph and recalculation**
- Edges run `formula node -> nodes read`. Reading an array or object depends on its structure and children.
- An edit marks dependents dirty; they are topologically sorted and evaluated.
- Cycles produce `#CIRCULAR!` on every node in the cycle.
- Dynamic dependencies (`JSONPATH` results, computed indices) are re-captured on every evaluation.

**Result checking**
1. The Python result converts to `Value`: `list` -> array, `dict` -> object, `int` / `float` / `str` / `bool` / `None` map directly; proxies are unwrapped.
2. The `Value` is validated against the node's `TypeSpec`: kind, required fields, exact field names, constraints.
3. Success: cached and displayed. Failure: the node shows an error value.

**Scalar types.** Conversion is strict: a result must already be the right kind, and nothing is coerced. If the author wants a different kind, they convert explicitly (`round`, `int`, `str`, `.isoformat()`, ...).

| Node type | Accepts | Rejects (`#TYPE!`) |
|---|---|---|
| `integer` | `int` | `float` (even `3.0`), `bool`, `str` |
| `number` / `decimal` / `double` | `int`, `float` | `bool`, `str` |
| `boolean` | `bool` | `int`, `float`, `str` |
| `string` | `str` | everything else |
| XSD `date` | `datetime.date` | `datetime.datetime`, `str` |
| XSD `dateTime` | `datetime.datetime` | `datetime.date`, `str` |

- `bool` is rejected where numbers are expected, though it is an `int` subclass in Python.
- An `int` is valid for a `number` node. A `float`, even a whole one, is not valid for an `integer` node; the author uses `round`, `floor`, `ceil`, or `int`.
- Integers outside the signed 64-bit range, and `NaN` or infinity, give `#VALUE!`.
- **Dates differ between the two languages, by design.**
  - **XML nodes** typed `date` / `dateTime` hold dates as `Value::Date` / `Value::DateTime`. They read in Python as `datetime.date` / `datetime.datetime` and accept those directly. A timezone-aware `datetime` keeps its offset and a naive one stays naive.
  - **JSON nodes** are always plain strings, even with `format: date` or `date-time`. They read as `str` and accept only `str`, so the author converts, e.g. `my_datetime.isoformat()`. The `format` keyword is still checked as a constraint (the string must be valid ISO 8601), but never changes the node's kind.

**Error values** are first-class: `#TYPE!` (path plus "expected X, got Y"), `#REF!`, `#NAME!`, `#VALUE!`, `#DIV/0!`, `#CIRCULAR!`, `#SYNTAX!` (the formula does not parse), `#PYERR!` (with traceback), `#TIMEOUT!`, `#SOURCE!` (a data source failed), `#UNTRUSTED!`. Errors propagate to dependents unless handled by `IFERROR`. Each carries a message shown in a tooltip.

**Errors inside Python.** A node holding an error value is read from Python as an `ErrorValue` object, not as an exception. Python's own exceptions stay the mechanism for failure, and the `ErrorValue` carries the one that caused it.
- `ErrorValue` has `.exception` (the original exception instance, with its traceback), `.kind` (e.g. `"#DIV/0!"`), `.message`, and `.path` (the node where the error originated). It is the same object whether the node errored directly or by propagation.
- **Navigation passes the error along; use raises it.**
  - Attribute access, indexing, and calling on an `ErrorValue`, or navigating below an errored node's proxy, return the same `ErrorValue`. A chain such as `Orders.items[0].price` therefore ends in an `ErrorValue` rather than failing midway.
  - Any real *use* of the value raises a `PropagatedError`: arithmetic, comparison, `bool()`, iteration, `len()`, `str()` / formatting, hashing, and conversion to `int` / `float`. `PropagatedError` has the original exception as `__cause__`, and maps to the original's kind (so a `ZeroDivisionError` upstream still shows `#DIV/0!` downstream). Its message names the origin path.
  - Because `sum()` and comprehensions use values, `SUM(Orders.items.price)` fails if any item's price is an error. **Default aggregations** (`SUM`, `AVERAGE`, `MIN`, `MAX`, `COUNT`, ...) always propagate errors this way. None of them silently skips bad items.
- **Custom error-skipping aggregations.** Skipping is something the author opts into by writing a function in a workbook module. Two helpers make that short:
  - `ISERROR(x)` tells whether an element is an `ErrorValue`.
  - `VALID(iterable)` yields the elements that are not errors, in order.

  ```python
  def SUM_VALID(values):
      return sum(VALID(values))

  def AVERAGE_VALID(values):
      good = list(VALID(values))
      return sum(good) / len(good)   # ZeroDivisionError -> #DIV/0! if none are valid
  ```

  A function that needs more (for example counting the skipped items) can use `ISERROR` directly, or `try/except PropagatedError`. Custom functions are ordinary public callables in a module, so they are available in formulas like any built-in.
- **A formula that returns an `ErrorValue`** (for example `=Orders.total` where the source is in error) puts that same error in its own node. Errors reach dependents without any code having to handle them.
- **`IFERROR(value, fallback)`**
  - `value` may be a plain reference: `IFERROR(Orders.total, 0)` returns `0` if `Orders.total` is an `ErrorValue`.
  - `value` may be a callable, which is evaluated inside a `try`: `IFERROR(lambda: Orders.total / Orders.count, 0)`. Any exception, including `PropagatedError` and Python errors like `ZeroDivisionError`, gives the fallback. Python evaluates arguments eagerly, so a lambda is the way to guard an expression rather than a single reference.
  - `ISERROR(value)` follows the same two forms and returns a `bool`.
- **Catching in Python.** `try: ... except PropagatedError as e:` works, and `e.__cause__` is the original exception. `except Exception` also catches it. This is the escape hatch inside custom functions, which can inspect `.kind` and `.exception`.
- Reading an errored node still records the dependency, so a node recalculates when its upstream error is fixed.
- The same rules apply to form button procedures and report expressions. An error reaching a report block shows in that block.

**Array and object formulas**
- An array node's formula must return a list; the result replaces its children as computed (read-only) children.
- An object node's formula returns a dict whose keys match the schema exactly.
- A formula on a node makes its whole subtree computed; the UI greys it out and rejects edits.
- Array items may carry their own formulas only when the array itself is not computed.

## Semantics: Python first, Excel second

A formula is a Python expression and the library behaves as Python programmers expect. Excel names are a convenience, not a compatibility contract.

| Area | Rule |
|---|---|
| Naming | Excel-style uppercase names (`SUM`, `IF`, `VLOOKUP`, ...) are thin aliases with Python-flavoured behaviour. Plain Python (`sum()`, `len()`, comprehensions, `x if c else y`) is equally first-class. |
| Truthiness / equality | Python's. `0`, `""`, `[]`, `None` are falsy; `==` is Python `==`; no case-insensitive string compare by default. |
| Empty / missing | `None` is the empty value. `SUM([])` is `0`; `AVERAGE([])` raises `ZeroDivisionError` (shown as `#DIV/0!`); no implicit `0` / `""` coercion. |
| Types | No implicit string-to-number coercion: `"1" + 1` is `#TYPE!`. `int` and `float` stay distinct; `/` is true division. |
| Indexing | 0-based, negative indices and slices allowed. Out of range raises, shown as `#REF!`. |
| Ranges / arrays | Python lists and iterables. No implicit array broadcasting (use comprehensions or `MAP`). |
| Strings | Python `str` with f-strings, `.format`, `%`. `TEXT` uses Python format specs, not Excel format codes. |
| Dates | Python `date` / `datetime` objects, not serial numbers. XML `date` / `dateTime` nodes read and accept them directly; JSON nodes are strings, so the author converts explicitly (e.g. `.isoformat()`). See Scalar types. |
| Lookups | `VLOOKUP`-style functions take lists of objects and field names. Missing keys raise. `#N/A` only from explicit `NA()`. |
| Errors | `ZeroDivisionError` -> `#DIV/0!`; `KeyError` / `IndexError` / `AttributeError` on a reference -> `#REF!`; `TypeError` -> `#TYPE!`; `ValueError` -> `#VALUE!`; `NameError` -> `#NAME!`; `SyntaxError` -> `#SYNTAX!`; anything else -> `#PYERR!`. Traceback goes in the tooltip. |
| Function list | Not a parity target. v1 ships a small Excel-named set that is useful over trees: `SUM`, `AVERAGE`, `MIN`, `MAX`, `COUNT`, `IF`, `IFERROR`, `VLOOKUP`, `FILTER`, `SORT`, `UNIQUE`, `CONCAT`, `ISERROR`, `VALID`, `TEXTJOIN`, `JSONPATH`, `XMLPATH`. Everything else comes from the Python standard library modules that work in the engine (`math`, `datetime`, `re`, `json`, `itertools`, ...; see Python engine). |

**Array concatenation**: `CONCAT(*arrays)` returns one list holding the items of each argument in order, e.g. `CONCAT(Orders.items, Archive.items)`. Arguments may be lists or array proxies; a non-array argument is a `#TYPE!` error. In the Python-first spirit, `CONCAT` means list concatenation, not text joining (use `TEXTJOIN` or `str.join` for text). Plain Python also works: array proxies support `+` and unpacking (`[*a, *b]`). On an array node, the result must still match the node's item type, so a mix of incompatible item types fails result checking.

Most built-ins are one-line Python over the stdlib. Only the tree-aware pieces (`JSONPATH`, `XMLPATH`, proxies) need real Rust.

**Extensibility**: custom functions live in Python modules stored in the workbook (see Python modules). Every module is available to formulas without an `import`, and its public functions can be called by bare name.

## Names and reserved words

Formulas, form button procedures, and report expressions share one flat namespace per workbook. It holds tab names (tree sheets and reports), data source names, module-level functions, built-in functions, Python builtins, the special names `this`, `parent`, `root`, `ErrorValue`, and `PropagatedError`, workbook module names, and the exposed stdlib modules (`math`, `datetime`, ...). Every name in it must be unique and mean the same thing everywhere. **Shadowing is not permitted**, with one exception (below).

**Reserved names.** These cannot be used as tab names, report names, data source names, or module-level function names:
- Python keywords (`keyword.iskeyword`). `class` passes `isidentifier()` but is not usable.
- All Python builtins (`list`, `sum`, `len`, `str`, ...).
- The special names above and the exposed stdlib module names.
- Every built-in function name (`SUM`, `IF`, `CONCAT`, `ISERROR`, `VALID`, ...).
- Any name starting with an underscore or with `cliotree`, which are held for internal use.

**Module functions.** Public callables in workbook modules are imported into the globals (see Extensibility). A function whose name clashes with a tab, a built-in, a reserved name, or a function in another module is an error at module load. The message names both parties. Nothing silently overrides anything.

**Case.** Names are case-sensitive in Python, but **tab names must also be unique ignoring case**: `orders` and `Orders` cannot coexist. They are confusing, and tab data is stored as per-tab files in the workbook, which Windows would treat as the same file. Functions and other names stay case-sensitive.

**Where it is checked**
- On creating or renaming a tab: live in the name field, naming the clash.
- On saving or loading a module.
- On opening a file. A file that breaks the rules still opens. The offending tab or function is flagged, anything depending on the clashing name fails with `#NAME!`, and the issues panel explains.

**Future built-ins.** The file records the reserved-names version it was created against. If a later release adds a built-in whose name an existing tab already uses, the tab keeps its name and **shadows** the newer built-in in that workbook, and a warning tells the author. This is the only permitted shadowing; it stops an upgrade from breaking old workbooks. New workbooks use the current list, with no exception.

## Python modules

Custom Python lives in **modules** stored in the workbook. The design follows Excel and its Visual Basic modules: a separate code editor with a project tree, one module per code window, and public functions that cells can call by name.

**The code editor** (opened from the menu or with Alt+F11, as in Excel) is the code area that also holds form designs.
- **Project explorer** on the left: a tree with a *Modules* branch listing the workbook's modules, and a *Forms* branch listing form designs (see Forms). Double-click opens an item in the editor.
- **Editor panes** on the right, one tab per open module: a monospace editor with line numbers, syntax highlighting, and automatic indentation.
- **Procedure box** above the editor, like the VBE's: a drop-down listing the module's top-level functions, which jumps to the one chosen.
- **Insert > Module** creates a module, and rename and delete are in the explorer's context menu. **Export** and **Import** write and read a module as a plain `.py` file, the way Excel exports a `.bas`.
- **Compile** checks the open module and lists problems (syntax errors, import cycles) with line and column, each clickable.
- **Immediate window** at the bottom, like the VBE's: type a Python expression, press Enter, and see its result or error. It runs against the live workbook on the evaluation thread, under the same budgets, and needs the workbook to be trusted.
- There is no debugger (breakpoints, stepping) in v1.

**Modules are loaded for every formula.** The formula context behaves as though it had imported all of the workbook's modules.
- A module is available **by name**, like an imported module: `Tax.vat(Orders.total)`.
- Its **public functions** (names not starting with an underscore) are also callable by **bare name**, like Excel's public functions: `vat(Orders.total)`. This is the same flat namespace described in Names and reserved words, so a clash is an error, not a silent override.
- The same applies in form button procedures, report expressions, and the Immediate window.

**Modules importing each other.** A module may `import` other workbook modules (and the allowed standard library modules). **Circular imports are not allowed.**
- Import statements are read from the module source when it is saved or loaded. If the modules form a cycle, the whole cycle fails with an error that names the cycle (`A -> B -> A`). None of the modules in it are loaded, and calling their functions gives `#NAME!`.
- Modules are loaded in dependency order.
- An import of something that exists neither as a workbook module nor as an available standard library module is a load error for that module.

**Loading and errors**
- Top-level statements in a module run once, when the module is loaded, under a load budget. Modules are for definitions (functions, constants), not for effects.
- A module with a syntax error, an import error, or an exception at load is marked failed (red in the explorer) and is listed in the error panel. Its functions are unavailable, and formulas that use them show `#NAME!` (with a message pointing at the failed module). Other modules are unaffected.
- A module's functions see the workbook the same way formulas do: they can read sheets such as `Orders.items` by name, and the reads are recorded as dependencies of whichever formula is running. This is looked up when the function is called, not when the module is loaded.

**Dependencies and recalculation.** When a formula uses a name that a module provides, it records a dependency on that module. Editing a module reloads it and recalculates exactly the formulas that depend on it, which show the pending marker until the results arrive.

**Names and storage.** A module name is a valid identifier, unique ignoring case, not a reserved word, and distinct from every tab and data source name. Modules are stored as `modules/<name>.py` in the `.clio` file. Their order in the explorer is recorded in the manifest.

**Editing, undo, and trust.** Module edits are logged for undo and redo (rapid typing is merged), and the double-underscore check applies to module source. Module sources are executable content, so in an untrusted workbook they are shown and can be edited but are not loaded, and the Immediate window is disabled.

## Reports

> **Deferred to a later version.** Reports are not part of v1. This section records the intended design so that v1 does not rule it out. Other sections that mention reports (names, trust, evaluation, undo, file format) describe how they will fit in, and apply once reports exist.

A **report** is a tab that takes data from other tabs and presents it visually: tables, text, and charts such as bar, line, pie, and scatter. A report holds no data of its own and is never the target of a formula or form.

**Report design** specifies:
- **Sources**: one or more tree sheets the report reads. Each source is exposed to the report's expressions under its sheet name, through the same proxies formulas use.
- **Layout**: a vertical flow of blocks (optionally in rows or columns). Block kinds:
  - `text`: static text or an f-string-style template filled from expressions.
  - `table`: rows from an array (or any list) with chosen columns, each column an expression over the row.
  - `chart`: a chart type, plus series. Each series gives expressions for its x and y values (or label and value for pie). Titles, axis labels, and colours are settable.
  - `group`: a titled container, optionally repeated once per item of a list (e.g. one section per customer).
- **Expressions** are Python, evaluated like formulas. Block data comes from expressions such as `[o.total for o in Orders.items]` or `AVERAGE(...)`.

The design is stored as a JSON document validated against another built-in meta-schema (`builtin:report`), like forms. A report tab's content is that design document, and a report designer is a visual editor over it.

**Behaviour**
- A report is live. Its expressions record dependencies like formulas, so editing a source node re-renders only the affected blocks.
- An error in an expression is shown in place, in the block that failed, as an error value (`#REF!`, `#TYPE!`, ...). Other blocks still render.
- A chart series must resolve to numeric (or date) values for numeric axes and to strings for labels. A mismatch shows `#TYPE!` in the chart area.
- Reports are read-only for the data they show. They cannot write to any sheet.
- A report's name is a Python identifier like any tab, but no other formula can reference it.
- Export (image, PDF, HTML) is a later feature and is not in v1.

## Forms

A **form** is a user-designed data-entry screen over one tab. It is not a tab itself: it has no tree grid and holds no data of its own. It is a different view of the data in the tab it is bound to.

**Form design** specifies:
- **Bound tab**: the name of a tree sheet. The form has the same schema as that tab, so there is no separate output schema. The binding is by name and is literal, like a formula reference: renaming or removing the tab leaves the form unbound and showing an error naming the missing tab (see Renames do not rewrite formulas). Several forms may bind to the same tab.
- **Layout**: a tree of widgets (fields, groups, repeaters, labels). Each field widget binds to a path in the bound tab's schema and takes its editor from the `TypeSpec` (toggle, dropdown, numeric, text, date). Repeaters bind to array nodes, groups to objects. A field may have a label and help text.
- **Buttons**: zero or more. Each has a label, an optional enable condition, and a Python procedure (the name of a function in a workbook module). Different buttons can name different procedures, and several buttons may share one.

**Where forms live in the UI**
- Forms are opened from a **Forms** menu, which lists the workbook's forms. Opening one shows the fill-in view.
- Forms are **designed in the same area as the custom Python modules**: a code area that holds modules and form designs side by side, each with its own editor. The form designer is a visual editor over the design document, and the area also offers the raw document for direct editing.
- The design is stored as a JSON document validated against a built-in schema (`builtin:form`), so it is diff-friendly and versioned with the workbook. A design is valid only if its field paths all exist in the bound tab's schema.

**Data flow: forms read and write the tab directly**
- **Retrieve.** When a form is opened, and whenever the tab's data changes, the form shows the tab's current values. It always reflects the last calculated values (see Evaluation and scheduling).
- **Replace on change.** When the user commits a change in a field (on Enter, or when the field loses focus), the new value **replaces** the corresponding node's data in the tab. The edit is an ordinary tab edit: it is undoable and triggers normal recalculation. A form does not hold a draft or a submit step.
- **Validation.** A field validates against its `TypeSpec` as the user types. An invalid value stays in the field with its error and is **not** written; the tab keeps the previous value.
- **Computed nodes are read-only.** A field bound to a node carrying a formula, or inside a computed subtree, is shown read-only with the computed styling. Edits cannot overwrite a formula result.
- Arrays are edited through repeaters: adding, removing, and editing items change the array in the tab directly.

**Button procedures**

A button runs a Python function `def my_procedure(form, ctx): ...` defined in a workbook module.
- `form` is a view of the bound tab's data. Reading it gives the current values. Changes made to it are collected during the call.
- `ctx` offers read access to the whole workbook through the same sheet proxies that formulas use, and it reports which button was clicked (`ctx.button`), so one function can serve several buttons.
- Procedures **obtain and send data to and from wherever needed**: they can read other tabs and `form`, and they can write to the bound tab through `form` or to other tabs through `ctx`. They can also use named data sources the user has granted (see Data sources). Writes to tabs are collected during the call and applied **atomically** when it returns.
- Every written value is validated against the schema of the tab it targets. If any write is invalid, or the procedure raises an exception, **none** of the writes are applied, the message is shown to the user, and the data is unchanged. Writing into a computed (formula-driven) node is refused in the same way.
- The applied writes form one undoable edit and trigger normal recalculation.
- Return values are ignored.
- Buttons run one at a time on the evaluation thread. While one runs, all buttons on the form are disabled and the status bar shows it.
- Procedures run under the same trust rules and resource limits as formulas. In an untrusted workbook buttons are disabled; fields still edit the tab directly, since that runs no Python.

## Data sources

A **data source** is a named connection to something outside the workbook. v1 supports:
- **Databases**: SQLite (a file), PostgreSQL, and MariaDB.
- **HTTP / HTTPS endpoints**: a base URL.

Data sources are the only way Python code reaches outside the workbook. The sandbox still exposes no `socket`, `subprocess`, `os`, or file I/O modules; everything external goes through a named source and its explicit grant.

**Definition.** A source has a name, a kind, and connection details (SQLite path; database host, port, and name; endpoint base URL; default headers). The definition is stored in the workbook, in a Data sources panel. **Secrets (passwords, tokens, API keys) are never stored in the workbook file.** They live in the operating system's credential store under the source name, and the workbook holds only a reference. Opening a workbook on another machine asks the user to supply them again.

**Names.** A source name is a Python identifier in the shared formula namespace, so it follows the rules in Names and reserved words and is available in formulas, button procedures, and report expressions as a global object.

**Explicit grants.** Every source must be **explicitly granted by the user, one by one**. There is no blanket "allow network" setting.
- A grant is recorded in user config, keyed by the workbook id, the source name, and a hash of the source's connection definition (kind, host, port, database or path, base URL). Changing where a source points invalidates its grant, so an edited file cannot redirect an already-granted name to a different server.
- A grant has a level, **read** or **read-write**, chosen by the user when granting. The grant dialog states the exact target (for example `postgres://db.example.com:5432/sales`) and the level.
- Granting needs the workbook itself to be trusted first. In an untrusted workbook no Python runs, so no source can be reached.
- Grants are listed and revoked in settings and in the Data sources panel.
- Grants are made only in that panel or settings. No dialog ever appears in the middle of an evaluation, because evaluation runs on a background thread. A call to an ungranted source fails with `#UNTRUSTED!` and a message naming the source.

**Using a source from Python**
- Databases: `Sales.query(sql, params)` returns a list of dicts, one per row. `Sales.execute(sql, params)` runs a statement that changes data and returns the number of rows affected. Parameters are always passed separately from the SQL text; there are no string-building helpers.
- HTTP: `Api.get(path, params=None)` and, for read-write grants, `Api.post`, `Api.put`, `Api.patch`, `Api.delete`. A response has `.status`, `.headers`, `.text`, and `.json()`.
- Values convert to Python types: integers, floats, strings, booleans, `date` / `datetime`, and `None` for SQL `NULL`. Result checking applies as usual when such a value reaches a node.

**Read versus read-write is enforced by the connection, not by parsing SQL**
- SQLite is opened read-only; PostgreSQL and MariaDB sessions use read-only transactions.
- With a read grant, `execute` and non-read HTTP methods are refused. A read-write grant is required for them.

**Where each kind of call may be used**
- **Reads** (`query`, `get`) may be used in formulas, button procedures, and report expressions.
- **Writes** (`execute`, `post`, `put`, `patch`, `delete`) are allowed **only in button procedures**, because formulas and reports re-run automatically and must not have side effects. Writes happen immediately during the call; they are not part of the atomic tab-write rule for procedures (see Forms), so a procedure that fails after a write does not undo it.
- Reads in formulas are slow and not deterministic, so they are treated as **volatile**: their results are cached for a recalculation pass keyed by (source, call, parameters) and refresh only on a recalculation pass or explicit "Recalculate" (see Evaluation and scheduling). Editing a source node does not by itself re-run a query, unless the query's parameters depend on it.

**Limits.** Waiting for I/O does not count against the per-formula CPU budget, but every call has its own timeout (default 30 seconds) and a maximum response size (default 16 MB), both workbook settings (see Settings and limits). A source failure (connection, authentication, SQL error, HTTP error status, timeout, oversize response) produces a new error kind, `#SOURCE!`, with the details in the message.

**Web port** *(later version)*. Browsers cannot open raw database connections, so SQLite, PostgreSQL, and MariaDB sources will be unavailable on the web. HTTP/HTTPS works through the browser's `fetch` and is subject to CORS.

## Trust model

**What trust covers.** Trust applies to all **executable content**: formulas, module sources, form button procedure names, and report expressions. The trust hash is a digest of a canonical serialisation of exactly that content. Data edits do not affect it.

**Recording trust.** User config stores, per workbook, a random **workbook id** (also stored in the file) and the **content hash** at the time of trust.
- **Edits made inside the app** to executable content (typing a formula, editing a module, pasting) update the stored hash while the workbook is trusted. No re-prompt.
- **On open**, a matching id and hash opens trusted. A matching id with a different hash means the file changed outside the app, so it opens untrusted, and the banner lists what changed (which formulas, modules, buttons, report expressions) so the user can review before re-trusting.
- **Save As** gives the copy a new id and carries the trust over, since the user made it.
- Copying an id into a hostile file does nothing, because the hash must match as well.
- A new or unknown file is untrusted. The banner offers "Trust this version" or continue read-only. Trusted workbooks can be listed and revoked in settings.

**Untrusted mode**
- Data, stored values, and stored errors display, and data editing works. Formula cells show the stored value with an `#UNTRUSTED!` badge.
- Formulas can be typed or edited but are not evaluated. Dependents of anything the user edits are shown as **stale** (greyed), since they cannot recalculate. Entering a formula prompts for trust; refusing keeps the text unevaluated.
- Form buttons are disabled, since they all run Python. Form fields still edit the tab directly.
- Reports show blocks that need no Python (static text) and an `#UNTRUSTED!` placeholder elsewhere.

**What containment can and cannot do.** Trust is the real security boundary, and the trust banner says plainly that trusting a workbook allows it to run code. Containment is defence in depth against accidents and casual abuse, not a determined attacker, because sandboxing Python is notoriously leaky:
- The Python engine is built without any operating-system access: no `os`, `subprocess`, `socket`, or file I/O modules exist in it (see Python engine for what this rests on). External access is only through named data sources, each separately granted by the user (see Data sources). Trusting a workbook does not by itself grant any source.
- `open`, `exec`, `eval`, `compile`, and `__import__` are removed from the builtins formulas see, and double-underscore attribute access is rejected when a formula is entered. These catch mistakes; they are not a boundary (see Python engine).

**Resource limits.** Evaluation takes a **budget**: a deadline and optionally a step count. Exceeding it aborts the evaluation with a new error kind, `#TIMEOUT!`, distinct from other Python errors. The limits are user settings (defaults: 1 second per formula and 10 seconds per recalculation; see Settings and limits). A memory limit is not promised in v1. How the budget is enforced is described under Python engine.

## Python engine

- **RustPython**, embedded behind a small `PythonEngine` trait: `eval(expr, globals) -> Result<PyValue>` and `load_module(src)`. The trait keeps the engine swappable (e.g. PyO3 + CPython on desktop later).
- Pure Rust, so the same code compiles to wasm for the web port, with no Python install required.
- Known limits: slower than CPython, incomplete stdlib, no C extensions (no numpy). RustPython describes itself as not totally production-ready, so the engine trait matters.

**Spike results.** A spike was run in October 2026 against RustPython 0.6.1 (GitHub head), on Windows with rustc 1.99.1. It built the interpreter with `--no-default-features --features freeze-stdlib`, which leaves out the `host_env` feature (the part that gives Python access to the operating system). The findings below replace the earlier assumptions in this section.

**Building and depending on it**
- RustPython 0.6.0 from crates.io **did not build** from a fresh dependency resolution: its `ruff` parser crates resolve to 0.16.10, which breaks the code generator, and pinning them to 0.16.5 breaks the parser. RustPython 0.5.0 fails on a `malachite-bigint` version clash. On Windows, `rustpython-host_env` also needed its `ctypes` feature.
- It built only from the repository's own `Cargo.lock`, which uses a git dependency on RustPython's `ruff` fork. On Windows that checkout failed with "path too long" and needed a clone with `core.longpaths` and a `[patch]` to a local copy.
- It needs **rustc 1.97 or newer** (the stable toolchain on the test machine had to be updated from 1.94).
- Consequence: depend on RustPython by pinned git revision, together with its lock file, and vendor or mirror the `ruff` fork. Do not rely on a plain `rustpython = "0.6"` dependency (see Build and dependencies for how it is vendored). Expect to re-check this on every upgrade, and keep the `PythonEngine` trait.

**Cancelling a running formula (the budget)**
- The mechanism works: a watchdog thread sends a closure through `user_signal_channel`, and the VM raises it inside the running code. A `while True: pass` loop and a loop of builtin calls were both stopped about 300 ms after a 300 ms deadline, with the exception surfacing as an ordinary Python error. The interpreter was **reusable afterwards**.
- **A single long builtin call cannot be interrupted.** `sum(range(10**10))` ignored the signal for as long as the test ran (it was killed externally after 20 seconds), because the signal is only checked between bytecode instructions. So the budget cannot catch everything.
- Mitigation: when a cancellation does not take effect within a grace period, abandon the evaluation thread and start a fresh interpreter, accepting that the old thread runs until the call finishes. Also guard the obvious cases (for example `range` above a size limit). The abandon path is the real backstop on desktop, and the number of abandoned threads is limited by a setting (default 20); see Settings and limits.
- On **wasm32** there is no second thread, so the Web Worker termination described earlier remains the only mechanism.

**What the sandbox really is**
- With `host_env` left out, the interpreter has **no file, socket, subprocess, or OS modules at all**: `os`, `posix`, `nt`, `socket`, `subprocess`, `ctypes`, `sqlite3`, `threading`, `pathlib`, `shutil`, `urllib`, and `http` all fail to import. `_io` imports but cannot open files ("Couldn't get FileIO").
- **Removing builtins is not a boundary.** With `open`, `__import__`, `eval`, `exec`, and `compile` removed from the builtins a formula sees, direct use fails with `NameError`. But `().__class__.__base__.__subclasses__()` still reaches `BuiltinImporter`, and `create_module(find_spec('sys'))` hands back `sys.modules`. Had `host_env` been compiled in, the same route would reach file I/O.
- So the guarantee comes from **what is compiled in**, not from what is hidden. Decisions that follow:
  - The Python engine is always built **without `host_env`** (and without `ssl`, sockets, and similar features). All access to files, databases, and the network happens in Rust, through the data source layer.
  - Trimming builtins is kept as a courtesy that catches mistakes, but it is not described as security.
  - As an extra layer, formulas are scanned when entered and rejected if they use double-underscore attribute access (`__class__`, `__subclasses__`, ...). The same scan applies to module source, with a short list of allowed dunder names (`__init__`, `__name__`, ...).

**Modules that work**
- Import fine without `host_env`: `math`, `datetime`, `re`, `json`, `itertools`, `decimal`, `fractions`, `collections`, `functools`, `string`, `textwrap`, `time`, `io`, `sys`.
- **`statistics` and `random` fail** (`no os specific module found`), because they import `os`. They are therefore **not available in v1**, and no replacement is planned. The built-in aggregates (`SUM`, `AVERAGE`, ...) cover the common cases, and a workbook module can supply anything else.
- Registering only an allowlist of native modules was not tested, because nothing dangerous is compiled in. It remains an option for trimming size.

**Speed (release build, Windows)**
- One formula over a 10,000-item list of objects (`sum(o['price'] * o['qty'] for o in items)`): **about 3.6 ms** per evaluation.
- 10,000 separate formulas, each precompiled and run in a fresh scope: **about 20 ms in total** (about 2 microseconds each).
- Compiling each formula from source every time: **about 147 ms** for 10,000 (about 15 microseconds each). So **compiled code should be cached per formula**, but even uncached it is not a bottleneck.
- Not measured: the real application reads data through proxy objects with lazy member access and dependency recording, which will be slower than plain dicts. This needs measuring when step 3 is built.

**Web (wasm32)** *(later version; recorded for planning)*
- RustPython's own `rustpython_wasm` crate **builds** for `wasm32-unknown-unknown`, which needs the `getrandom` `wasm_js` feature (the crate sets it).
- The unoptimised output was **54.6 MB**. That build includes features we would not use (`ssl`, documentation strings) and was not passed through `wasm-opt` or compression. The achievable size is not yet known and must be measured with a minimal feature set.
- Python was not run in a browser, so Web Worker termination remains untested.

**Not covered by the spike:** running under `wasm32` in a real browser, the proxy layer, memory use, and the effect of the `threading` feature.

## Evaluation and scheduling

All Python evaluation (formulas, form button procedures, report expressions) runs on a **background evaluation thread**, never on the egui UI thread. On the later web port the same role would be played by a Web Worker, using the same message protocol.

**The UI never waits.** Every cell is drawn from its **last calculated value**, the one stored in the tree (see Stored values). A cell whose evaluation is outstanding keeps showing that value, with a subtle pending marker, until a new result arrives. A cell that has a formula but has never been calculated shows empty with the pending marker. The same applies to reports: they render the last values and refresh as results arrive.

**Status bar.** A status bar along the bottom always shows evaluation state:
- `Ready` when nothing is outstanding.
- `Calculating... N pending` while evaluations are queued or running, optionally with a progress indicator.
- Counts of errors and invalid cells, and the workbook's trust state.

**Message flow**
1. An edit on the UI thread marks the affected formula nodes dirty and sends them to the evaluation thread.
2. The evaluation thread evaluates dirty nodes in dependency order and posts results (value or error value, plus the dependencies read) back.
3. The UI applies results at the start of a frame and requests a repaint. Dependents of a changed result are queued in turn.

**Consistency**
- Every result is tagged with the input version it was computed from. If an edit has since changed something the formula read, the result is discarded and the formula is queued again. The UI never shows a result computed from superseded inputs once it has been superseded.
- A formula never observes a half-applied edit: the evaluation thread works against a consistent snapshot of the data and picks up newer versions between evaluations.
- An in-flight evaluation whose inputs have changed may be cancelled through the same mechanism as the timeout budget (see Python engine), so a long-running formula does not delay the response to an edit.
- Per-formula and per-recalculation budgets apply as described in the trust model; a timeout is stored as `#TIMEOUT!` like any other error.

**Volatile functions.** `NOW`, `RAND` and similar are evaluated once per recalculation pass and keep that value until the next pass. A full recalculation happens when a trusted workbook is opened and when the user asks for it explicitly ("Recalculate", F9). There is no timer-driven refresh in v1.

**Saving while evaluating.** Saving waits for outstanding evaluations to finish so the file never holds half-updated values. The status bar shows that it is waiting, and the user can cancel the wait and save with the current last values, which are flagged as pending in the overlay.

**Forms.** Button procedures run on the same thread, one at a time (as already specified). While one runs, the form's buttons are disabled and the status bar shows it.

## Settings and limits

Limits are not hard-coded. The user sets them, with defaults, in two places.

- **Workbook settings** are stored in the workbook (`settings.json`) and edited in a *Workbook settings* panel. They govern how that workbook's Python and data sources run.
- **Application preferences** belong to the user, not to any file. They hold the limits that apply *before* a workbook is open or trusted (reading a file, importing), and a **ceiling** for each workbook setting.

**A workbook cannot raise its own limits past the user's ceilings.** A file is untrusted input, so a hostile workbook that asks for a one-hour time limit gets the ceiling instead. The panel shows each value with its default and ceiling, rejects out-of-range input, and shows the effective value when a ceiling applies. Workbook settings are part of the executable content covered by trust, since they govern execution (see Trust model); editing them inside the application keeps trust, and a change made outside it drops trust. Changes are undoable edits.

**Workbook settings**

| Setting | Default | Meaning |
|---|---|---|
| Formula time limit | 1 s | Longest a single formula, button procedure, or module load step may run (CPU time, not I/O waits) before `#TIMEOUT!`. |
| Recalculation time limit | 10 s | Longest one recalculation pass may run. |
| Cancel grace period | 500 ms | After a cancel signal, how long to wait for the evaluation to stop before abandoning it. |
| **Maximum abandoned threads** | **20** | How many abandoned evaluations (see below) may still be running at once. |
| Maximum range length | 10,000,000 | Largest sequence `range` may produce. |
| Data source call timeout | 30 s | Longest a data source call may take, including waiting for I/O. |
| Maximum response size | 16 MB | Largest data source response accepted. |

More settings can be added in the same way, with a default and a ceiling each.

**Application preferences** (not stored in workbooks): the ceiling for each workbook setting; and limits that apply while a file is being read or imported, namely the maximum number of zip entries, maximum total uncompressed size, maximum import file size, and maximum nesting depth; plus the undo history bounds (entry count and memory budget).

**Abandoned threads.** A formula that does not stop after a cancel signal and the grace period (for example one long builtin call, which cannot be interrupted; see Python engine) is **abandoned**: its result is discarded, its node gets `#TIMEOUT!`, and a fresh interpreter takes over so evaluation continues. The abandoned thread keeps running until its call returns, and then it is released. The number of such threads still running is limited by *Maximum abandoned threads*.
- The count appears in the status bar when it is above zero (for example `2 abandoned`).
- **At the limit, evaluation is suspended.** No new evaluation starts, pending cells keep their pending marker, and the status bar shows `Evaluation suspended: 20 abandoned threads still running`. It resumes by itself when the count drops below the limit.
- The panel offers **Save and restart**, which saves the workbook (without waiting for evaluation, flagging pending cells as for any save) and restarts the application, since a thread that never returns can only be cleared that way.
- Raising the limit takes effect immediately, up to the user's ceiling.

**Guards on builtins.** The `range` builtin is replaced by a version that refuses a length above *Maximum range length*, raising `ValueError` (shown as `#VALUE!`) with a message naming the setting. This catches the accidental case such as `sum(range(10**10))` before it starts. Guards are **best effort**, not a security boundary: they cover the wrapped builtins and obvious literals, not every way to build a huge value. The abandoned-thread limit is the real backstop, and a memory limit is not promised in v1.

## Undo and redo

The workbook keeps one **edit log** so that user edits can be undone and redone. The history is a single linear sequence for the whole workbook, like Excel's, not one per tab.

**What is logged.** Every edit the user makes:
- literal values, and structural changes (inserting, removing, moving, or sorting items and map entries);
- formulas, and notes;
- tabs: adding, removing, renaming, reordering, and rebinding schemas;
- Python modules, form designs, report designs, and data source definitions (not secrets and not grants);
- schema sheet edits, which are ordinary tree edits;
- the table-wide edits made by paste, import, and button procedures.

**What is not logged.**
- Computed results, error values, and pending state. They are derived, not authored. After an undo, the formulas affected are recalculated and the new results appear as usual.
- View settings such as expanded nodes, column widths, and whether an array is shown as a table.
- Effects outside the workbook. A procedure that writes to a database or HTTP endpoint cannot be undone; its workbook edits can.
- Granting or revoking data-source access, and trust decisions.

**Entries.**
- One user gesture is one entry: committing a field, a paste, running a button procedure (all of its writes together), an import, dragging items into a new order.
- Rapid consecutive edits to the same text cell during one editing session are merged.
- Each entry has a short description, shown in the Edit menu ("Undo Set Orders.items[2].price").
- An entry records enough to reverse itself: the old and new value, or the removed subtree. Removed subtrees are held by reference, so deleting a large array does not copy it, and restoring a subtree restores its original `NodeId`s.
- Entries refer to nodes by `NodeId`, not by path, so they stay valid when other edits shift paths. Formula entries refer to a location (a path), consistent with formulas belonging to a location.

**Behaviour**
- Undo and redo are ordinary edits as far as the rest of the system is concerned. They apply on the UI thread, bump the data version, and queue recalculation. Evaluation results computed from superseded inputs are discarded by the usual version rule.
- A new edit after an undo clears the redo history. There is no branching.
- Undoing or redoing a change to executable content (a formula, module, button, or report expression) counts as an edit made in the app, so it keeps the workbook's trust (see Trust model).
- Undo and redo are disabled while a button procedure is running.
- The log tracks the saved state, so undoing back to the saved point clears the "modified" mark.
- The history lives in memory only. It is not saved in the `.clio` file and is cleared when the workbook is closed. It is bounded by an entry count and a memory budget (application preferences), and the oldest entries are dropped first.
- Keys: Ctrl+Z to undo, Ctrl+Y or Ctrl+Shift+Z to redo.

**Structure.** `cliotree-core` defines the primitive edit operations on the data tree and how to apply and invert them. The workbook-level log, which also covers the other kinds of edit above, lives in the layer above (`cliotree-calc`), and the UI only issues edits and requests undo and redo.

## Tabular view of arrays

An array whose items are objects can be shown as a table instead of as indented tree rows.
- **Toggling.** The user right-clicks the array node's row. A context menu offers **Tabular view**, a toggle that switches it on or off. The menu item appears only on arrays of objects.
- **Persistence.** The setting is per array node and saved in the tab's overlay, as view state. It is not part of the data, not undoable, and not part of the trust hash.

**What the table shows**
- One **row** per item, with the item index in a row header.
- One **column** per scalar field of the item type, in the schema's declared order, with the field name as the header.
- A field that is itself an object is **flattened** into dotted columns (`address.city`), to any depth.
- A field that is an array or a map has a **summary cell** (for example `[5 items]`). Opening it drills into that value as an ordinary tree below the table, so tables do not nest.
- Optional fields that are absent appear as empty cells that can be filled.

**Cells are the same cells as in the tree.** A table cell is the node's cell, with all the same behaviour: literal editing with the type-aware editors, formulas, error values, the pending marker, the invalid-data outline, and notes. Selecting one shows its formula in the formula bar. Formulas refer to nodes by path as always (`Orders.items[2].price`); the table is a different way of drawing the tree, not different data.

**Editing in the table**
- Add a row at the end, insert above or below, delete rows, and move rows up or down. These are structural edits and are undoable.
- A formula on the array node itself makes the whole array computed, so the table is read-only.
- Clicking a column header sorts the **display** only, and the row header still shows each item's real index. Because references like `items[2]` are literal, the table never reorders data implicitly. An explicit command, "Apply this order to the data", turns the current display order into a real reorder.
- Cell ranges copy and paste as tab-separated text. A paste validates each cell and is applied as one undoable entry; cells that don't validate are rejected with a message and nothing is written.

**Performance.** Both the table and the tree view only draw the rows that are on screen, so arrays of many thousands of items stay responsive. The table can be built on `egui_extras`' virtualised table support.

## Import and export

**Importing a standalone file** creates new tabs in the workbook: a data tab, and a **schema sheet** holding the schema inferred from the data. The data tab is bound to that schema sheet by the normal sheet-reference rule.

| File | Data tab format | Inferred schema |
|---|---|---|
| `.json` | JSON | JSON Schema |
| `.xml` | XML | XSD |
| anything else (plain text) | JSON | JSON Schema for an array of strings |

- **Plain text** imports as though it were a JSON array of strings, one per line. Line endings are normalised, a final newline does not add an empty item, and blank lines in the middle are kept as empty strings. The text must be UTF-8 (a BOM is stripped); anything else is refused with a message.
- **Alternative to inference.** The import dialog also offers "bind to an existing schema sheet". The data is then loaded against that schema, and anything that doesn't match follows the normal invalid-data rules (kept, flagged, unrecognised content preserved).
- **Names.** The tabs are named from the file name, sanitised to a valid identifier (invalid characters become `_`; a leading digit, a leading underscore, a reserved word, or a clash with an existing name gets a prefix or numeric suffix). The schema sheet is `<Name>Schema`. The dialog lets the user change both before importing.
- **Import report.** A dialog shows the inferred schema and every warning (dropped nulls, type conflicts, oversized integers, a dropped DOCTYPE) before the user confirms. Nothing is added to the workbook until then.
- Parsing is bounded: limits on file size and nesting depth guard against hostile input (application preferences; see Settings and limits). Importing never runs Python.

**Schema inference** always produces a schema inside the v1 subset (closed objects, arrays, simple types, no unions or nulls). It is a starting point; the user can edit the schema sheet afterwards.

| Observed data | Inferred schema |
|---|---|
| Object | Object with properties in observed order. A property present in every observed instance is `required`; otherwise optional. |
| Array of objects | One item type; properties merged across all items, required only if present in every item. |
| Scalars | `string`, `integer`, `number`, or `boolean`. A mix of integers and non-integers is `number`. |
| Empty array | Items typed `string`, and flagged in the report as "inferred from no data". |
| `null` | Cannot be represented (no nulls in v1). Treated as absent: the property becomes optional; a null array item is dropped. Both are reported. |
| JSON strings | Plain `string`. Date-like strings are not given a `format`. |

- **Conflicts.** If the same position holds different kinds of value, the **most common kind wins** (a tie goes to the more general: string over number over integer over boolean). Values of the other kinds are not converted or discarded: they stay in the tree as invalid literals, flagged. A conflict between structures (an object where most instances have a string, or vice versa) keeps the minority subtree as unrecognised content, flagged.
- **XML** specifics:
  - The root element becomes the single global element. Nested elements are anonymous types.
  - An element that repeats under its parent anywhere in the data becomes an array (`maxOccurs="unbounded"`); an attribute becomes `xs:attribute`, required if always present.
  - Element text is typed by its values: integers → `integer`, decimals → `decimal`, `true`/`false` → `boolean`, ISO 8601 dates and datetimes → `date` / `dateTime`, otherwise `string`. Values with leading zeros (other than `0`) are strings, so identifiers like `007` are not turned into numbers.
  - Child order is the merge of the orders seen across instances; if instances disagree irreconcilably, the first-seen order is used and the report says so.
  - **Refused with a clear message:** mixed content (non-whitespace text next to child elements), and more than one namespace. These are outside the v1 XSD subset.

**Exporting.** Any tree tab exports as a standalone file in its own format.
- Everything the user gave the tab is written: field order, array order, XML attributes and elements as the schema maps them, XML comments (from notes), namespace prefix and XML declaration, and **unrecognised content** that was preserved on import. The result is what the tab shows, including the latest computed values. Errored nodes are absent (see Stored values).
- Dates are written in their XML lexical forms, floats with their shortest round-trip spelling, and JSON/XML indentation is two spaces by default with a compact option.
- Formulas, notes in JSON, and tab settings are not part of a standalone export; the `.clio` is the format that keeps them.
- A schema sheet exports as a standalone `.json` or `.xsd`/`.xml` file in the same way, and the export dialog offers to include the schema alongside the data.
- A tab whose root is an array of strings can also be exported as a plain text file, one string per line, mirroring text import.

## File format

A workbook is saved as a **zip archive with the extension `.clio`**. It is an ordinary zip, so any zip tool can open it, and the data inside is plain JSON, XML, and Python source.

**Layout**

```
manifest.json            first entry in the archive
sheets/<Tab>/data.json   (or data.xml)   the tab's data tree, with stored values
sheets/<Tab>/overlay.json                formulas and their status
reports/<Report>.json                    report design (later version; reserved)
forms/<Form>.json                        form design
modules/<name>.py                        Python modules
datasources.json                         data source definitions, no secrets
settings.json                            workbook settings (see Settings and limits)
```

- **Tabs** are one directory each. Tab names are valid identifiers and unique ignoring case (see Names and reserved words), so they are safe as file names on every platform. A schema sheet is stored the same way as any tree sheet (JSON or XML data, according to its meta-schema).
- **Data files** hold the plain data, including the latest computed values, exactly as other tools would expect (see Stored values). Errored nodes are absent from them.
- **Overlay** holds, per node path: the formula text, and its status (ok, pending, or the error kind, message, and traceback). Entries are sorted by path. It also holds the tab's notes (for JSON tabs) and its view state, such as which arrays are shown as tables.
- **Reports and forms** are the design documents described in their sections, with the report or form name as the file name.
- **Modules** are plain `.py` files, so they can be edited, diffed, and linted with ordinary tools.

**Manifest** (`manifest.json`) records:
- `format_version` (major.minor), and the application version that wrote the file.
- The **workbook id** used by the trust model.
- The **reserved-names version** (see Names and reserved words).
- The **Python engine and its version**, so a file written against one RustPython release can be flagged when opened with another.
- The list of tabs in tab order, each with its kind (`tree` in v1; `report` is reserved for the later version) and, for tree sheets, its **schema binding**: `builtin:jsonschema`, `builtin:xsd`, or `sheet:<Tab>`.
- The list of forms and modules, in display order.

**Writing**
- Text is UTF-8 with LF line endings. JSON is pretty-printed with a stable key order; data keeps the schema's declared field order.
- Archive entries are written in a fixed order, with a fixed timestamp and no per-save metadata. Saving a workbook whose content hasn't changed produces identical bytes, so a `.clio` file behaves well in version control, and unzipped it diffs cleanly.
- Saves are atomic: write to a temporary file in the same directory, then rename over the original.
- Saving waits for outstanding evaluation (see Evaluation and scheduling).

**Reading**
- The zip is treated as untrusted input. Entry paths that are absolute or contain `..` are rejected, and limits apply to the number of entries and to total uncompressed size (to guard against zip bombs; these are application preferences, see Settings and limits). Opening a file never runs anything; Python only runs after the trust check.
- **Versioning.** The same major version opens. A newer minor version opens with a warning, and unknown entries are kept and written back unchanged on save. A newer major version is refused with a message saying which version is needed. Older formats are upgraded in memory on open, and the user is warned before the upgraded file overwrites the original.
- The trust hash is computed from the parsed executable content (see Trust model), not from the bytes of the archive. Reordering entries or changing whitespace does not change it.

**Plain data.** Because data files are ordinary JSON or XML, users can extract a tab's data from the archive directly. Importing and exporting standalone files is described in Import and export.

## Application structure

Cargo workspace:
- `cliotree-core`: `Value`, `TypeSpec`, paths, schema lowering (JSON Schema, XSD), format adapters, validation. No UI, no Python.
- `cliotree-calc`: formula storage, dependency graph, recalculation, error values, proxy objects, `PythonEngine` trait and RustPython implementation.
- `cliotree-std`: built-in functions.
- `cliotree-app`: egui frontend and file I/O. The only crate touching `eframe` or the filesystem, so that a later wasm port needs only a different `main` and storage layer. Keeping these crates free of UI and filesystem code is a v1 design constraint for that reason.

**Workbook file format**: a zip with the extension `.clio` (see File format).

**UI**
- Sheet tabs along the bottom; formula bar above a tree grid. (In a later version, a report tab will show the rendered report instead, with charts drawn through `egui_plot`.)
- Tree grid columns: `Name | Type | Value | Formula`; expandable nodes, array items as indexed rows.
- Type-aware editors: toggles for booleans, dropdowns for enums, numeric fields.
- Schema violations get a red outline; computed cells get a distinct fill.
- Side panels for the error list, and for modules and trust state.
- A Forms menu that opens form-fill views, and a code area holding Python modules and form designs (see Forms).

## Testing and CI

The project has automated tests at several levels, and **all of them run in a CI pipeline on Linux** with no manual steps. A developer on Windows runs the same default tests locally. Tests that need extra services (databases) or a display are opt-in locally and always on in CI.

**Principles**
- **Test the real thing.** Application-level tests drive the actual application code, including the evaluation thread and the RustPython engine, not mocks. The Python engine is real in every test above the unit level.
- **No sleeping.** The status bar's pending count is observable (see Evaluation and scheduling), and the application exposes "wait until idle" for tests. Tests wait for that condition with a timeout, never for a fixed delay.
- **Deterministic.** The clock and random numbers are injected, so `NOW` and `RAND` can be fixed. Fonts used in rendered tests are bundled. Tests use temporary directories and never touch the network except to local servers they start.
- **Every rule in this document gets a test**, and every fixed bug gets a regression test. Test names refer to the section of this document that they check.

**Levels**

1. **Unit tests** (`cargo test`, per crate). `cliotree-core`: `Value`, `TypeSpec`, JSON Schema and XSD lowering, validation, scalar-type checks, the XML mapping, inference, import and export. `cliotree-calc`: the dependency graph, evaluation order, cycle detection, error values and propagation, version tagging of results, the edit log, the trust hash.
2. **Conformance and fixture tests.**
   - *JSON Schema:* a selected part of the official JSON Schema Test Suite, vendored at a pinned version. Only cases for keywords in the v1 subset are run as pass/fail checks. For every excluded keyword there is a test that checks it is **rejected at load with an error naming it**, which is what this document requires.
   - *XSD:* a curated corpus of schemas and instances covering the subset, and a test for each excluded construct. The built-in meta-schemas are validated by loading them through the normal path.
   - *Workbooks:* `.clio` fixtures in the repository, one per file-format version, for upgrade tests, plus hostile files (zip paths with `..`, zip bombs, DTD entity expansion, very deep nesting, oversized input).
3. **Data-driven formula tests.** A case file (schema, data, formulas, expected values or error kinds) is loaded and checked, in a table-driven format that is easy to add to. This covers the Python semantics table, error mapping, `IFERROR` and `ISERROR` in both forms, `CONCAT`, `VALID`, syntax errors, and literal references.
4. **Property-based tests** (`proptest`).
   - Data round-trips through JSON and XML, and through save and load.
   - Saving twice gives identical bytes.
   - A random sequence of edits followed by undoing all of them restores the original state, and redo restores the final one.
   - A tree inferred from random data validates it, apart from the conflicts the inference reports.
5. **Python engine tests.** The error mapping table; the propagation rules for `ErrorValue`; module loading, including import cycles, failed modules, and `#NAME!` results; timeouts, with a tight loop that is cancelled and a long builtin call that is abandoned; and the **sandbox probes from the spike** kept as permanent regression tests (no `os`, `socket`, or file access reachable, including through `BuiltinImporter`).
6. **In-process UI tests.** The application's UI is written so that its top-level `update` function can be driven by `egui_kittest`, which runs egui headlessly and finds widgets through AccessKit, the way a screen reader would. This needs no display. Tests do what a user does: open a workbook, select a node, type `=...` into the formula bar and press Enter, wait until idle, and check the cell's text, error, and pending marker. Cases include the tabular view toggle from the context menu, undo and redo keys, the trust banner and untrusted mode, forms (field commits, button procedures), the code editor and the Immediate window, import and export dialogs, and the status bar. This is the main level for UI behaviour.
7. **Visual snapshot tests** (`egui_kittest` with its `snapshot` and `wgpu` features). Key screens are rendered and compared with stored images with a small tolerance. On Linux CI the rendering uses a software Vulkan driver (Mesa lavapipe), so no GPU is needed. They run as a separate job and fail only on a clear difference; differences are uploaded as artifacts. Snapshots start as advisory and become a gate once they are stable.
8. **End-to-end tests on the running application.** The real binary is started on Linux under a virtual display (`xvfb-run`) and driven through a **test control channel**: a loopback-only JSON command interface that exists only when the application is built with the `test-control` cargo feature, which release builds never enable. Commands include: open a workbook, select a node, enter text, press a key, read a cell's value, formula, error, and pending state, wait until idle, take a screenshot, and quit. Because the channel is compiled out of release builds, it adds no remote-control surface to the shipped product. A small number of tests, written as a Rust client against that channel, cover startup, open and save, edit-and-recalculate, and shutdown on the real windowing backend. Most behaviour is tested at level 6, which is faster and more stable.
9. **Data source tests.** SQLite runs everywhere. PostgreSQL and MariaDB run in CI against service containers. HTTP uses a mock server started by the test on a local port. Cases include grants (an ungranted call gives `#UNTRUSTED!`), read versus read-write enforcement, `#SOURCE!` errors, timeouts, oversize responses, and a check that a saved `.clio` file contains no secrets. These are skipped locally unless the matching environment variable is set.
10. **Security and robustness.** The hostile-file fixtures above; the sandbox probes; trust-hash tests (a change outside the application drops trust, a change inside keeps it); and **fuzzing** with `cargo-fuzz` on the parsers: JSON and XML import, zip reading, schema lowering, and formula parsing. Fuzzing runs on a nightly schedule and does not block merges; any crash found becomes a regression test.
11. **Performance checks.** Benchmarks for the cases measured in the spike (a formula over 10,000 items, 10,000 separate formulas, loading and saving a large workbook) run on a schedule and report trends. They are not merge gates, because shared CI machines are noisy; a very generous threshold in an ordinary test catches only gross regressions.

**Pipeline** (GitHub Actions, since the repository is on GitHub)

| Job | Runs | Blocks merge |
|---|---|---|
| Lint | `cargo fmt --check`, `cargo clippy -D warnings`, `cargo deny` (licences, advisories) | yes |
| Test | `cargo test --workspace` on Linux: levels 1 to 6 | yes |
| Services | level 9 with PostgreSQL and MariaDB containers | yes |
| E2E | level 8 under `xvfb-run` | yes |
| Snapshots | level 7 with software rendering | advisory at first |
| Nightly | fuzzing, performance, `cargo audit` | no |

- The Rust toolchain is pinned in `rust-toolchain.toml` (1.97 or newer is required, as RustPython requires) and the dependency set is locked, including the RustPython git revision and the `ruff` fork (see Build and dependencies).
- Build caching (for example `Swatinem/rust-cache`) keeps runs short, because RustPython takes several minutes to build from scratch.
- On failure the job uploads logs, snapshot diffs, and screenshots as artifacts.
- Coverage is reported with `cargo llvm-cov` for information, not as a gate.
- Because development happens on Windows, a Windows job running levels 1 to 3 is added once the basic pipeline is stable. It exists to catch path, case-sensitivity, and line-ending differences, which this design has several rules about (tab names unique ignoring case, per-tab files).

## Build and dependencies

**Required Rust version.** The project requires **Rust 1.97 or newer**. This is a project requirement for every developer and for CI, not just a CI detail. It comes from RustPython and its `ruff` fork (see Python engine), and it is re-checked whenever they are upgraded.
- The workspace `Cargo.toml` declares `rust-version = "1.97"`, so Cargo itself refuses to build with an older compiler and says why.
- `rust-toolchain.toml` pins the exact toolchain used by developers and CI. It starts at 1.99.1, the version the spike was run with, and is raised deliberately, not automatically.
- The README states the requirement, and a developer on an older toolchain is told to update with `rustup update stable`.

**The vendor folder.** Dependencies that cannot come from crates.io as published (RustPython, and the `ruff` fork it needs) live in a `vendor/` folder at the root of the repository.
- **Not committed.** `vendor/` is in `.gitignore`. Its contents are downloaded, never stored in the repository.
- **Not cleared between builds.** An ordinary build, rebuild, or `cargo clean` leaves it alone, so a build does not download anything again.
- **Cleared by the project's `clean`.** The project's own clean command (`cargo xtask clean`) removes `vendor/` as well as the build output. After it, the next build fetches everything again.
- **Fetched on demand.** The build script checks that every required vendor item exists under `vendor/`. If one is missing, it calls a separate **fetch script**, which downloads the item and does any build steps it needs, and then the build carries on.

**What is pinned.** A committed file, `vendor.lock`, lists each vendor item with its source URL and **exact commit hash**. The content is not committed, but its identity is, so every developer and CI build gets the same code.
- Items at first: RustPython at the revision chosen after the spike, and the `ruff` fork at the revision that RustPython's own lock file names.
- The fetch script checks out that exact commit and verifies it against `vendor.lock`. A mismatch is an error.
- Each item gets a stamp file (`.complete`) holding the pinned hash. It is written only after the download and any build steps succeed, so an interrupted fetch is detected and redone. If `vendor.lock` changes, the stamp no longer matches and the item is fetched again.
- To upgrade, a developer runs `cargo xtask vendor update <item> <commit>`, which edits `vendor.lock` for review in the usual way.
- The URL in `vendor.lock` can point at a mirror if an upstream repository disappears.

**The fetch script is cross-platform.** It is a small Rust program in its own package (`tools/fetch-vendor`), so it runs the same on Windows, Linux, and in CI, with no shell scripting. The build script runs it with a separate target directory, to avoid deadlocking on Cargo's lock on the main one.

**Windows long paths.** The spike found that checking out the whole `ruff` repository fails on Windows because of very long test file names. The fetch script avoids the problem by **sparse checkout**: it fetches only the crates RustPython needs and none of the rest, and it sets git's `core.longpaths` as well. Nothing else about the layout depends on the platform.

**How Cargo uses it.** The workspace refers to the vendored sources by path (`vendor/rustpython/...`), and patches the `ruff` crates to `vendor/ruff/...`. All other dependencies come from crates.io in the usual way, locked by `Cargo.lock`.

**Offline and failure.** If a fetch fails, for example without network access, the build stops with a message that names the item, the URL, and the directory. The environment variable `CLIOTREE_VENDOR_DIR` can point at a copy of the vendor folder prepared elsewhere.

**CI.** The pipeline uses the same build script, so there is no separate CI procedure. The `vendor/` folder is cached, keyed by the hash of `vendor.lock`, and a cache miss simply fetches again (see Testing and CI).

**Security.** Fetching does not run anything from the downloaded repositories. Cargo does run the build scripts of dependencies when it builds them, as it does for any crate, which is why the pins are exact commits and changes to `vendor.lock` are reviewed.

## Build order

0. **CI skeleton**: workspace, `rust-version` and pinned toolchain, the `vendor/` folder with its fetch script and `vendor.lock`, lint and test jobs running on Linux (see Build and dependencies, and Testing and CI). Every later step adds its tests together with its code.
1. **core**: `Value`, `TypeSpec`, JSON Schema lowering, validation, tests.
2. **calc**: graph, evaluator, error values, with a trivial fake engine first.
3. **RustPython** integration, proxies, `JSONPATH`, and measuring the proxy layer's overhead against the spike's plain-dict figures.
4. **app**: tree grid, formula bar, tabs, the edit log with undo and redo, then the tabular view of arrays.
5. **std** functions, workbook modules, trust.
6. **XSD** subset and XML adapter.
6b. **Data sources**: definitions panel, credential store, grants, HTTP first, then SQLite, PostgreSQL, MariaDB.
7. **Forms**: `builtin:form` schema, form-fill view with live read and write to the tab, button procedures, then the designer.

Reports and the wasm port are **not part of v1**; see Later versions.

## Later versions

Deferred features, with what v1 does to keep them possible:

- **Reports** (see Reports): `builtin:report` schema, table and text blocks, then charts, then the designer.
  - The manifest's tab `kind` and the `reports/` directory name are reserved.
  - A v1 build that opens a file containing reports keeps those entries unchanged on save (the unknown-entries rule in File format) and shows no report tabs.
- **Web (wasm) port**: the same core in a browser.
  - `cliotree-core` and `cliotree-calc` stay free of UI and filesystem code.
  - Open points recorded for it: the binary size (54.6 MB unoptimised in the spike), running Python and terminating a Web Worker in a browser, file access, persistence of trust, credential storage, and database data sources being unavailable.

## Risks and open questions

- **RustPython speed**: per-element formulas over large arrays may be slow. Mitigations: vectorise common functions in Rust, cache bytecode per formula.
- **Cheap data snapshots**: letting the evaluation thread read a consistent snapshot while the UI keeps editing implies a data tree with cheap snapshots (for example reference-counted nodes with copy-on-write), or a message-based ownership model. This should be settled in step 1 when `Value` is defined.
- **Dependency granularity**: reading `Orders.items` is coarse. If too coarse, add path-prefix subscriptions.
- **Forms and XML**: XML schemas need the `@attr` / `#text` mapping to be reflected in form field paths. The designer should hide this and show attribute and element names.
- **Report rendering and export**: charts are drawn live in egui, so producing the same output as an image or PDF needs a second rendering path. That is deferred, and the layout model should stay renderer-neutral to allow it.
- **Button procedure power**: whether procedures should be able to write to other sheets (e.g. update a counter) is deliberately left out of v1.
- **Computed children**: the rule for formulas on array items inside computed arrays needs validating against real use.
