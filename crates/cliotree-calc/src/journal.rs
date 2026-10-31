//! Beancount journals as data sources (DESIGN.md, "Beancount journals").
//!
//! A definition names the journal's main file. It is read with its included
//! files (only from the main file's folder or below), checked the way
//! Beancount checks it as far as ClioTree goes (postings balance, accounts
//! are open, balance assertions hold, padding filled in), and kept until one
//! of the files changes. Python reads it through `source(name).transactions()`
//! and the other methods, and adds a transaction with `add()`, which appends
//! text to the end of the main file and changes nothing that is there.
//!
//! Amounts are exact decimals. They cross to Python as `{"$dec": "3.50"}` and
//! dates as `{"$date": "2026-01-02"}`, which the prelude turns into `Decimal`
//! and `date`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Instant, SystemTime};

use beancount_parser::{DirectiveContent, Entry, PostingPrice, parse_iter};
use rust_decimal::Decimal;
use serde_json::{Map, Value as Json, json};

use crate::sources::{SourceDef, SourceKind, TestOutcome};

/// When a file was last changed, as far as the journal is concerned: its
/// length and its modification time, or nothing if it is gone.
type Stamp = Option<(u64, SystemTime)>;

fn stamp(path: &Path) -> Stamp {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

/// A journal as it was read.
#[derive(Debug, Clone, Default)]
pub struct Journal {
    /// The main file, as the definition names it.
    pub main: PathBuf,
    /// The transactions in date order, padding included.
    pub transactions: Vec<Txn>,
    pub accounts: BTreeMap<String, AccountInfo>,
    pub prices: Vec<PriceRow>,
    /// What is wrong with the books, each with its file and line.
    pub errors: Vec<String>,
    pub options: Vec<(String, String)>,
    pub files: usize,
}

#[derive(Debug, Clone)]
pub struct Txn {
    pub date: String,
    pub flag: String,
    pub payee: Option<String>,
    pub narration: String,
    pub tags: Vec<String>,
    pub links: Vec<String>,
    pub meta: Vec<(String, Meta)>,
    /// The file, relative to the main file's folder, and the line.
    pub file: String,
    pub line: u32,
    pub postings: Vec<Post>,
    /// Made by a `pad` directive to make a balance assertion hold.
    pub padding: bool,
}

#[derive(Debug, Clone)]
pub enum Meta {
    Text(String),
    Number(Decimal),
    Currency(String),
}

#[derive(Debug, Clone)]
pub struct Post {
    pub flag: Option<char>,
    pub account: String,
    /// The units; filled in for the one posting that left them out.
    pub amount: Option<Decimal>,
    pub currency: Option<String>,
    pub cost: Option<Decimal>,
    pub cost_currency: Option<String>,
    pub cost_date: Option<String>,
    /// The price of one unit (a total price `@@` is divided out).
    pub price: Option<Decimal>,
    pub price_currency: Option<String>,
    pub meta: Vec<(String, Meta)>,
    /// The amount was left out and worked out from the other postings.
    pub auto: bool,
}

#[derive(Debug, Clone, Default)]
pub struct AccountInfo {
    pub open: Option<String>,
    pub close: Option<String>,
    pub currencies: Vec<String>,
    pub booking: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PriceRow {
    pub date: String,
    pub currency: String,
    pub amount: Decimal,
    pub quote: String,
}

/// What was read for one definition, and the files it came from, to notice
/// when one of them changes.
struct Loaded {
    watched: Vec<(PathBuf, Stamp)>,
    result: Result<Arc<Journal>, String>,
}

impl Loaded {
    fn fresh(&self) -> bool {
        self.watched.iter().all(|(path, s)| stamp(path) == *s)
    }
}

fn cache() -> &'static Mutex<HashMap<PathBuf, Loaded>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Loaded>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn main_path(def: &SourceDef) -> Result<PathBuf, String> {
    if def.kind != SourceKind::Beancount {
        return Err(format!(
            "{} is a {} source, not a Beancount journal",
            def.name,
            def.kind.label()
        ));
    }
    let path = def.path.as_deref().unwrap_or_default().trim();
    if path.is_empty() {
        return Err(format!(
            "{}: a Beancount source needs the path of its journal",
            def.name
        ));
    }
    Ok(PathBuf::from(path))
}

/// The journal of a definition: read again only when one of its files
/// changed since it was last read.
pub fn journal(def: &SourceDef) -> Result<Arc<Journal>, String> {
    let main = main_path(def)?;
    let mut cache = cache().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(loaded) = cache.get(&main)
        && loaded.fresh()
    {
        return loaded.result.clone();
    }
    let _waiting = crate::sources::Waiting::begin();
    let loaded = load(&main);
    let result = loaded.result.clone();
    cache.insert(main, loaded);
    result.map_err(|e| format!("{}: {e}", def.name))
}

/// Whether the journal was read and one of its files has changed since: the
/// formulas that read it are then out of date.
pub fn changed(def: &SourceDef) -> bool {
    let Ok(main) = main_path(def) else {
        return false;
    };
    let cache = cache().lock().unwrap_or_else(|e| e.into_inner());
    cache.get(&main).is_some_and(|loaded| !loaded.fresh())
}

fn forget(main: &Path) {
    let mut cache = cache().lock().unwrap_or_else(|e| e.into_inner());
    cache.remove(main);
}

// ---------------------------------------------------------------- reading

fn load(main: &Path) -> Loaded {
    let mut watched = Vec::new();
    let result = read(main, &mut watched).map(Arc::new);
    Loaded { watched, result }
}

/// A directive with where it came from.
struct Found {
    file: String,
    order: usize,
    directive: beancount_parser::Directive<Decimal>,
}

fn read(main: &Path, watched: &mut Vec<(PathBuf, Stamp)>) -> Result<Journal, String> {
    watched.push((main.to_owned(), stamp(main)));
    match std::fs::metadata(main) {
        Ok(m) if m.is_file() => {}
        Ok(_) => return Err(format!("{} is not a file", main.display())),
        Err(e) => return Err(format!("{} cannot be opened: {e}", main.display())),
    }
    let canonical =
        canonical(main).map_err(|e| format!("{} cannot be opened: {e}", main.display()))?;
    let root = canonical
        .parent()
        .map(Path::to_owned)
        .unwrap_or_else(|| canonical.clone());
    let mut journal = Journal {
        main: main.to_owned(),
        ..Journal::default()
    };
    let mut found: Vec<Found> = Vec::new();
    let mut pending = vec![canonical];
    let mut seen: HashSet<PathBuf> = HashSet::new();
    while let Some(path) = pending.pop() {
        if !seen.insert(path.clone()) {
            continue;
        }
        watched.push((path.clone(), stamp(&path)));
        let rel = relative(&root, &path);
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{rel} cannot be read: {e}"))?;
        for (i, line) in text.lines().enumerate() {
            if let Some(rest) = line.strip_prefix("plugin ") {
                journal.errors.push(format!(
                    "{rel}:{}: the plugin {} is not run by ClioTree, so what it would add or \
                     change is missing",
                    i + 1,
                    rest.trim()
                ));
            }
        }
        let dir = path.parent().map(Path::to_owned).unwrap_or_default();
        let mut includes = Vec::new();
        let mut lines_read: HashSet<u32> = HashSet::new();
        for entry in parse_iter::<Decimal>(&text) {
            match entry {
                Err(e) => {
                    return Err(format!(
                        "{rel}:{}: this is not Beancount syntax",
                        e.line_number()
                    ));
                }
                Ok(Entry::Directive(directive)) => {
                    lines_read.insert(directive.line_number);
                    found.push(Found {
                        file: rel.clone(),
                        order: found.len(),
                        directive,
                    });
                }
                Ok(Entry::Option(o)) => journal.options.push((o.name, o.value)),
                Ok(Entry::Include(include)) => {
                    let (files, dirs) = resolve(&root, &dir, &include)
                        .map_err(|why| format!("{rel}: include {}: {why}", include.display()))?;
                    for d in dirs {
                        watched.push((d.clone(), stamp(&d)));
                    }
                    includes.extend(files);
                }
                Ok(_) => {}
            }
        }
        // The parser passes over a line it does not know without a word; a
        // mistyped transaction would then be missing from the books.
        if let Some(line) = unread_line(&text, &lines_read) {
            return Err(format!(
                "{rel}:{line}: this line is not Beancount that ClioTree understands"
            ));
        }
        // The first include is read first.
        pending.extend(includes.into_iter().rev());
    }
    journal.files = seen.len();
    process(&mut journal, found);
    Ok(journal)
}

/// The first line at the start of a line that is neither something the
/// parser read nor something it is known to pass over: comments, org-mode
/// headings, the undated keywords, and the dated directives ClioTree does not
/// use (`note`, `document`, `custom`, `query`).
fn unread_line(text: &str, lines_read: &HashSet<u32>) -> Option<usize> {
    const UNDATED: [&str; 7] = [
        "option", "include", "plugin", "pushtag", "poptag", "pushmeta", "popmeta",
    ];
    const PASSED_OVER: [&str; 4] = ["note", "document", "custom", "query"];
    for (i, line) in text.lines().enumerate() {
        let n = i + 1;
        let Some(first) = line.chars().next() else {
            continue;
        };
        if first.is_whitespace() || line.trim().is_empty() || ";*#".contains(first) {
            continue;
        }
        let mut words = line.split_whitespace();
        let word = words.next().unwrap_or_default();
        if UNDATED.contains(&word) {
            continue;
        }
        if first.is_ascii_digit() {
            let keyword = words.next().unwrap_or_default();
            if lines_read.contains(&(n as u32)) || PASSED_OVER.contains(&keyword) {
                continue;
            }
        }
        return Some(n);
    }
    None
}

/// A path made absolute, without the `\\?\` prefix Windows gives it, after
/// which `/` and `..` in an include would not work.
fn canonical(path: &Path) -> std::io::Result<PathBuf> {
    let full = path.canonicalize()?;
    let text = full.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return Ok(PathBuf::from(format!(r"\\{rest}")));
    }
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        return Ok(PathBuf::from(rest));
    }
    Ok(full)
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// The files an `include` names, which may have `*` and `?` in them, and the
/// folders that were listed for them (a new file there changes the journal).
/// Only files in the main file's folder or below may be included.
fn resolve(
    root: &Path,
    dir: &Path,
    include: &Path,
) -> Result<(Vec<PathBuf>, Vec<PathBuf>), String> {
    let joined = if include.is_absolute() {
        include.to_owned()
    } else {
        dir.join(include)
    };
    let text = joined.to_string_lossy();
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    if !text.contains(['*', '?']) {
        files.push(canonical(&joined).map_err(|e| format!("cannot be opened: {e}"))?);
    } else {
        let mut places = vec![PathBuf::new()];
        for part in joined.components() {
            let name = part.as_os_str().to_string_lossy().into_owned();
            if !name.contains(['*', '?']) {
                for place in &mut places {
                    place.push(part.as_os_str());
                }
                continue;
            }
            let mut next = Vec::new();
            for place in &places {
                dirs.push(place.clone());
                let Ok(entries) = std::fs::read_dir(place) else {
                    continue;
                };
                let mut names: Vec<String> = entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| wildcard(&name, n))
                    .collect();
                names.sort();
                next.extend(names.into_iter().map(|n| place.join(n)));
            }
            places = next;
        }
        for place in places {
            if place.is_file() {
                files.push(canonical(&place).map_err(|e| format!("{e}"))?);
            }
        }
    }
    for file in &files {
        if !file.starts_with(root) {
            return Err(
                "an included file must be in the journal's folder or a folder below it".to_owned(),
            );
        }
    }
    Ok((files, dirs))
}

/// `*` for any run of characters and `?` for one.
fn wildcard(pattern: &str, name: &str) -> bool {
    fn go(p: &[char], n: &[char]) -> bool {
        match p.split_first() {
            None => n.is_empty(),
            Some(('*', rest)) => (0..=n.len()).any(|i| go(rest, &n[i..])),
            Some(('?', rest)) => !n.is_empty() && go(rest, &n[1..]),
            Some((c, rest)) => n.first() == Some(c) && go(rest, &n[1..]),
        }
    }
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    go(&p, &n)
}

fn date_text(d: &beancount_parser::Date) -> String {
    format!("{:04}-{:02}-{:02}", d.year, d.month, d.day)
}

fn meta_of(map: &beancount_parser::metadata::Map<Decimal>) -> Vec<(String, Meta)> {
    let mut out: Vec<(String, Meta)> = map
        .iter()
        .filter_map(|(k, v)| {
            let value = if let Some(s) = v.as_string() {
                Meta::Text(s.to_owned())
            } else if let Some(n) = v.as_number() {
                Meta::Number(*n)
            } else {
                Meta::Currency(v.as_currency()?.as_str().to_owned())
            };
            Some((k.to_string(), value))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Where a directive goes among those of its day: openings and balance
/// assertions first (an assertion is about the start of its day), closings
/// last.
fn rank(content: &DirectiveContent<Decimal>) -> u8 {
    match content {
        DirectiveContent::Open(_) => 0,
        DirectiveContent::Balance(_) => 1,
        DirectiveContent::Close(_) => 3,
        _ => 2,
    }
}

struct PadState {
    date: String,
    source: String,
    file: String,
    line: u32,
    order: usize,
    used: HashSet<String>,
}

fn process(journal: &mut Journal, mut found: Vec<Found>) {
    found.sort_by(|a, b| {
        (
            date_text(&a.directive.date),
            rank(&a.directive.content),
            a.order,
        )
            .cmp(&(
                date_text(&b.directive.date),
                rank(&b.directive.content),
                b.order,
            ))
    });
    let mut balances: HashMap<String, HashMap<String, Decimal>> = HashMap::new();
    let mut pads: HashMap<String, PadState> = HashMap::new();
    let mut txns: Vec<(usize, Txn)> = Vec::new();
    for item in found {
        let d = &item.directive;
        let date = date_text(&d.date);
        let at = format!("{}:{}", item.file, d.line_number);
        match &d.content {
            DirectiveContent::Open(open) => {
                let name = open.account.as_str().to_owned();
                let info = journal.accounts.entry(name.clone()).or_default();
                if info.open.is_some() {
                    journal
                        .errors
                        .push(format!("{at}: {name} is opened a second time"));
                } else {
                    info.open = Some(date);
                    let mut currencies: Vec<String> = open
                        .currencies
                        .iter()
                        .map(|c| c.as_str().to_owned())
                        .collect();
                    currencies.sort();
                    info.currencies = currencies;
                    info.booking = open.booking_method.as_ref().map(|b| b.to_string());
                }
            }
            DirectiveContent::Close(close) => {
                let name = close.account.as_str().to_owned();
                match journal.accounts.get_mut(&name) {
                    Some(info) if info.open.is_some() => info.close = Some(date),
                    _ => journal
                        .errors
                        .push(format!("{at}: {name} is closed but was never opened")),
                }
            }
            DirectiveContent::Price(price) => journal.prices.push(PriceRow {
                date,
                currency: price.currency.as_str().to_owned(),
                amount: price.amount.value,
                quote: price.amount.currency.as_str().to_owned(),
            }),
            DirectiveContent::Pad(pad) => {
                let account = pad.account.as_str().to_owned();
                for name in [&account, &pad.source_account.as_str().to_owned()] {
                    if let Some(why) = not_open(&journal.accounts, name, &date) {
                        journal.errors.push(format!("{at}: {why}"));
                    }
                }
                pads.insert(
                    account,
                    PadState {
                        date,
                        source: pad.source_account.as_str().to_owned(),
                        file: item.file.clone(),
                        line: d.line_number,
                        order: item.order,
                        used: HashSet::new(),
                    },
                );
            }
            DirectiveContent::Balance(assertion) => {
                let account = assertion.account.as_str().to_owned();
                let currency = assertion.amount.currency.as_str().to_owned();
                let expected = assertion.amount.value;
                if let Some(why) = not_open(&journal.accounts, &account, &date) {
                    journal.errors.push(format!("{at}: {why}"));
                }
                let mut actual = sum_under(&balances, &account, &currency);
                if let Some(pad) = pads.get_mut(&account)
                    && !pad.used.contains(&currency)
                {
                    pad.used.insert(currency.clone());
                    let difference = expected - actual;
                    if !difference.is_zero() {
                        let posting = |account: &str, amount: Decimal| Post {
                            flag: None,
                            account: account.to_owned(),
                            amount: Some(amount),
                            currency: Some(currency.clone()),
                            cost: None,
                            cost_currency: None,
                            cost_date: None,
                            price: None,
                            price_currency: None,
                            meta: Vec::new(),
                            auto: false,
                        };
                        let txn = Txn {
                            date: pad.date.clone(),
                            flag: "P".to_owned(),
                            payee: None,
                            narration: format!(
                                "(Padding inserted for Balance of {expected} {currency} for \
                                 difference {difference} {currency})"
                            ),
                            tags: Vec::new(),
                            links: Vec::new(),
                            meta: Vec::new(),
                            file: pad.file.clone(),
                            line: pad.line,
                            postings: vec![
                                posting(&account, difference),
                                posting(&pad.source, -difference),
                            ],
                            padding: true,
                        };
                        apply(&mut balances, &txn);
                        txns.push((pad.order, txn));
                        actual = expected;
                    }
                }
                let tolerance = assertion
                    .tolerance
                    .unwrap_or_else(|| half_unit(expected.scale()));
                if (actual - expected).abs() > tolerance {
                    journal.errors.push(format!(
                        "{at}: the balance of {account} is {actual} {currency}, not \
                         {expected} {currency} as asserted"
                    ));
                }
            }
            DirectiveContent::Transaction(t) => {
                let mut tags: Vec<String> = t.tags.iter().map(|x| x.as_str().to_owned()).collect();
                tags.sort();
                let mut links: Vec<String> =
                    t.links.iter().map(|x| x.as_str().to_owned()).collect();
                links.sort();
                let postings = t
                    .postings
                    .iter()
                    .map(|p| {
                        let units = p.amount.as_ref();
                        let (price, price_currency) = match &p.price {
                            Some(PostingPrice::Unit(a)) => {
                                (Some(a.value), Some(a.currency.as_str().to_owned()))
                            }
                            Some(PostingPrice::Total(a)) => {
                                let per = units
                                    .filter(|u| !u.value.is_zero())
                                    .map(|u| a.value / u.value.abs());
                                (per, Some(a.currency.as_str().to_owned()))
                            }
                            None => (None, None),
                        };
                        let cost = p.cost.as_ref();
                        Post {
                            flag: p.flag,
                            account: p.account.as_str().to_owned(),
                            amount: units.map(|u| u.value),
                            currency: units.map(|u| u.currency.as_str().to_owned()),
                            cost: cost.and_then(|c| c.amount.as_ref()).map(|a| a.value),
                            cost_currency: cost
                                .and_then(|c| c.amount.as_ref())
                                .map(|a| a.currency.as_str().to_owned()),
                            cost_date: cost.and_then(|c| c.date.as_ref()).map(date_text),
                            price,
                            price_currency,
                            meta: meta_of(&p.metadata),
                            auto: false,
                        }
                    })
                    .collect();
                let mut txn = Txn {
                    date: date.clone(),
                    flag: t.flag.map_or("*".to_owned(), |f| f.to_string()),
                    payee: t.payee.clone(),
                    narration: t.narration.clone().unwrap_or_default(),
                    tags,
                    links,
                    meta: meta_of(&d.metadata),
                    file: item.file.clone(),
                    line: d.line_number,
                    postings,
                    padding: false,
                };
                if let Err(why) = complete(&mut txn) {
                    journal.errors.push(format!("{at}: {why}"));
                }
                let mut reported = HashSet::new();
                for p in &txn.postings {
                    if reported.insert(p.account.clone())
                        && let Some(why) = not_open(&journal.accounts, &p.account, &date)
                    {
                        journal.errors.push(format!("{at}: {why}"));
                    }
                }
                apply(&mut balances, &txn);
                txns.push((item.order, txn));
            }
            _ => {}
        }
    }
    txns.sort_by(|a, b| (a.1.date.as_str(), a.0).cmp(&(b.1.date.as_str(), b.0)));
    journal.transactions = txns.into_iter().map(|(_, t)| t).collect();
}

/// Half of the last digit of a number with `scale` decimals: how far apart
/// two amounts written that precisely may be.
fn half_unit(scale: u32) -> Decimal {
    Decimal::new(5, scale + 1)
}

fn under(account: &str, top: &str) -> bool {
    account == top
        || account
            .strip_prefix(top)
            .is_some_and(|rest| rest.starts_with(':'))
}

fn sum_under(
    balances: &HashMap<String, HashMap<String, Decimal>>,
    account: &str,
    currency: &str,
) -> Decimal {
    balances
        .iter()
        .filter(|(name, _)| under(name, account))
        .filter_map(|(_, by)| by.get(currency))
        .sum()
}

fn apply(balances: &mut HashMap<String, HashMap<String, Decimal>>, txn: &Txn) {
    for p in &txn.postings {
        if let (Some(amount), Some(currency)) = (p.amount, &p.currency) {
            *balances
                .entry(p.account.clone())
                .or_default()
                .entry(currency.clone())
                .or_default() += amount;
        }
    }
}

/// Why `account` may not be used on `date`, if it may not.
fn not_open(accounts: &BTreeMap<String, AccountInfo>, account: &str, date: &str) -> Option<String> {
    let info = accounts.get(account);
    match info.and_then(|i| i.open.as_deref()) {
        None => Some(format!("{account} is used but never opened")),
        Some(open) if date < open => {
            Some(format!("{account} is used before it is opened on {open}"))
        }
        _ => match info.and_then(|i| i.close.as_deref()) {
            Some(close) if date > close => {
                Some(format!("{account} is used after it was closed on {close}"))
            }
            _ => None,
        },
    }
}

/// Fills in the posting that left out its amount, and checks that the
/// postings balance. A posting held at a cost that is not given (`{}`, which
/// Beancount books against earlier lots) is not checked, since ClioTree does
/// not book lots.
fn complete(txn: &mut Txn) -> Result<(), String> {
    let mut sums: BTreeMap<String, (Decimal, Decimal)> = BTreeMap::new();
    let mut open = Vec::new();
    let mut unknown = false;
    for (i, p) in txn.postings.iter().enumerate() {
        let (Some(units), Some(currency)) = (p.amount, p.currency.as_ref()) else {
            open.push(i);
            continue;
        };
        let (weight, weight_currency, scale) = if let Some(cost_currency) = &p.cost_currency {
            match p.cost {
                Some(cost) => (
                    units * cost,
                    cost_currency.clone(),
                    units.scale().max(cost.scale()),
                ),
                None => {
                    unknown = true;
                    continue;
                }
            }
        } else if p.cost_date.is_some() {
            unknown = true;
            continue;
        } else if let (Some(price), Some(price_currency)) = (p.price, &p.price_currency) {
            (
                units * price,
                price_currency.clone(),
                units.scale().max(price.scale()),
            )
        } else {
            (units, currency.clone(), units.scale())
        };
        let entry = sums.entry(weight_currency).or_default();
        entry.0 += weight;
        entry.1 = entry.1.max(half_unit(scale));
    }
    if open.len() > 1 {
        return Err(format!(
            "{} postings leave out their amount; only one may",
            open.len()
        ));
    }
    if unknown {
        return Ok(());
    }
    if let Some(&i) = open.first() {
        let left: Vec<(String, Decimal)> = sums
            .into_iter()
            .filter(|(_, (sum, _))| !sum.is_zero())
            .map(|(c, (sum, _))| (c, -sum))
            .collect();
        let template = txn.postings[i].clone();
        let filled: Vec<Post> = left
            .into_iter()
            .map(|(currency, amount)| Post {
                amount: Some(amount),
                currency: Some(currency),
                auto: true,
                ..template.clone()
            })
            .collect();
        if !filled.is_empty() {
            txn.postings.splice(i..=i, filled);
        }
        return Ok(());
    }
    let off: Vec<String> = sums
        .iter()
        .filter(|(_, (sum, tolerance))| sum.abs() > *tolerance)
        .map(|(c, (sum, _))| format!("{sum} {c}"))
        .collect();
    if off.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "the postings do not balance: {} left over",
            off.join(" and ")
        ))
    }
}

// --------------------------------------------------------------- queries

/// A number as it was written: `3.50` stays `3.50`.
fn dec(d: Decimal) -> Json {
    json!({ "$dec": d.to_string() })
}

fn day(d: &str) -> Json {
    json!({ "$date": d })
}

fn opt<T>(value: Option<T>, f: impl Fn(T) -> Json) -> Json {
    value.map_or(Json::Null, f)
}

fn meta_json(meta: &[(String, Meta)]) -> Json {
    let map: Map<String, Json> = meta
        .iter()
        .map(|(k, v)| {
            let v = match v {
                Meta::Text(s) | Meta::Currency(s) => Json::String(s.clone()),
                Meta::Number(n) => dec(*n),
            };
            (k.clone(), v)
        })
        .collect();
    Json::Object(map)
}

fn txn_json(t: &Txn) -> Json {
    let postings: Vec<Json> = t
        .postings
        .iter()
        .map(|p| {
            json!({
                "flag": opt(p.flag, |f| Json::String(f.to_string())),
                "account": p.account,
                "amount": opt(p.amount, dec),
                "currency": p.currency,
                "cost": opt(p.cost, dec),
                "cost_currency": p.cost_currency,
                "cost_date": opt(p.cost_date.as_deref(), day),
                "price": opt(p.price, dec),
                "price_currency": p.price_currency,
                "meta": meta_json(&p.meta),
                "auto": p.auto,
            })
        })
        .collect();
    json!({
        "date": day(&t.date),
        "flag": t.flag,
        "payee": t.payee,
        "narration": t.narration,
        "tags": t.tags,
        "links": t.links,
        "meta": meta_json(&t.meta),
        "file": t.file,
        "line": t.line,
        "padding": t.padding,
        "generated": opt(generated(&t.narration), |k| Json::String(k.to_owned())),
        "postings": postings,
    })
}

fn arg<'a>(args: &'a Json, name: &str) -> Option<&'a str> {
    args.get(name)
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// A read: `transactions`, `balance`, `accounts`, `prices` or `errors`.
pub fn read_call(def: &SourceDef, method: &str, args: &Json) -> Result<Json, String> {
    let j = journal(def)?;
    match method {
        "transactions" => {
            let account = arg(args, "account");
            let since = arg(args, "since");
            let until = arg(args, "until");
            let payee = arg(args, "payee").map(str::to_lowercase);
            let tag = arg(args, "tag");
            let link = arg(args, "link");
            let found: Vec<Json> = j
                .transactions
                .iter()
                .filter(|t| account.is_none_or(|a| t.postings.iter().any(|p| under(&p.account, a))))
                .filter(|t| since.is_none_or(|s| t.date.as_str() >= s))
                .filter(|t| until.is_none_or(|u| t.date.as_str() <= u))
                .filter(|t| {
                    payee
                        .as_deref()
                        .is_none_or(|p| t.payee.as_deref().is_some_and(|x| x.to_lowercase() == p))
                })
                .filter(|t| tag.is_none_or(|g| t.tags.iter().any(|x| x == g)))
                .filter(|t| link.is_none_or(|l| t.links.iter().any(|x| x == l)))
                .map(txn_json)
                .collect();
            Ok(Json::Array(found))
        }
        "balance" => {
            let account = arg(args, "account").ok_or("balance() needs the name of an account")?;
            let until = arg(args, "until");
            let mut sums: BTreeMap<String, Decimal> = BTreeMap::new();
            for t in &j.transactions {
                if until.is_some_and(|u| t.date.as_str() > u) {
                    continue;
                }
                for p in &t.postings {
                    if let (true, Some(amount), Some(currency)) =
                        (under(&p.account, account), p.amount, &p.currency)
                    {
                        *sums.entry(currency.clone()).or_default() += amount;
                    }
                }
            }
            Ok(Json::Object(
                sums.into_iter()
                    .filter(|(_, v)| !v.is_zero())
                    .map(|(c, v)| (c, dec(v)))
                    .collect(),
            ))
        }
        "accounts" => Ok(Json::Array(
            j.accounts
                .iter()
                .map(|(name, info)| {
                    json!({
                        "account": name,
                        "open": opt(info.open.as_deref(), day),
                        "close": opt(info.close.as_deref(), day),
                        "currencies": info.currencies,
                        "booking": info.booking,
                    })
                })
                .collect(),
        )),
        "prices" => {
            let currency = arg(args, "currency");
            Ok(Json::Array(
                j.prices
                    .iter()
                    .filter(|p| currency.is_none_or(|c| p.currency == c))
                    .map(|p| {
                        json!({
                            "date": day(&p.date),
                            "currency": p.currency,
                            "amount": dec(p.amount),
                            "quote": p.quote,
                        })
                    })
                    .collect(),
            ))
        }
        "errors" => Ok(json!(j.errors)),
        "options" => Ok(Json::Object(
            j.options
                .iter()
                .map(|(k, v)| (k.clone(), Json::String(v.clone())))
                .collect(),
        )),
        _ => Err(format!(
            "{method} is not something a Beancount source can do"
        )),
    }
}

// --------------------------------------------------------------- writing

/// The kinds of transaction ClioTree writes. A mistake is never edited or
/// deleted: it is corrected by an adjustment or a reversal added after it.
pub const GENERATED: [&str; 3] = ["original", "adjustment", "reversal"];

const MARK_START: &str = "[ClioTree generated ";

/// The narration with the mark that says ClioTree wrote the transaction:
/// `Invoice #10765 [ClioTree generated original transaction]`.
pub fn mark(narration: &str, kind: &str) -> String {
    let mark = format!("{MARK_START}{kind} transaction]");
    match narration.trim() {
        "" => mark,
        text => format!("{text} {mark}"),
    }
}

/// The kind of transaction ClioTree wrote, if its narration ends with the
/// mark.
pub fn generated(narration: &str) -> Option<&'static str> {
    let at = narration.rfind(MARK_START)?;
    let rest = &narration[at + MARK_START.len()..];
    GENERATED
        .into_iter()
        .find(|kind| rest.trim_end() == format!("{kind} transaction]"))
}

fn number(value: &Json, what: &str) -> Result<Option<Decimal>, String> {
    let text = match value {
        Json::Null => return Ok(None),
        Json::Object(o) if o.contains_key("$dec") => {
            o["$dec"].as_str().unwrap_or_default().to_owned()
        }
        Json::String(s) => s.trim().to_owned(),
        Json::Number(n) => n.to_string(),
        other => return Err(format!("{what} is not a number: {other}")),
    };
    text.parse::<Decimal>()
        .map(Some)
        .map_err(|_| format!("{what} is not a number: {text}"))
}

fn text_field<'a>(value: &'a Json, what: &str) -> Result<Option<&'a str>, String> {
    match value {
        Json::Null => Ok(None),
        Json::String(s) => {
            if s.contains(['\n', '\r']) {
                Err(format!("{what} cannot have a line break in it"))
            } else {
                Ok(Some(s.as_str()))
            }
        }
        other => Err(format!("{what} is not text: {other}")),
    }
}

fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn valid_date(date: &str) -> bool {
    let parts: Vec<&str> = date.split('-').collect();
    let [y, m, d] = parts[..] else {
        return false;
    };
    let (Ok(y), Ok(m), Ok(d)) = (y.parse::<u32>(), m.parse::<u32>(), d.parse::<u32>()) else {
        return false;
    };
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    date.len() == 10 && (1000..=9999).contains(&y) && (1..=days).contains(&d)
}

fn valid_currency(c: &str) -> bool {
    let first = c.chars().next();
    first.is_some_and(|f| f.is_ascii_uppercase())
        && c.chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || "'._-".contains(ch))
        && c.len() <= 24
}

fn valid_word(w: &str) -> bool {
    !w.is_empty() && w.chars().all(|c| c.is_alphanumeric() || "-_/.".contains(c))
}

fn valid_key(k: &str) -> bool {
    k.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && k.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// `add()`: checks one transaction against the journal and appends it to the
/// end of the main file. The caller has checked *Allow writes* and that this
/// is a procedure or the Immediate window.
pub fn append(def: &SourceDef, request: &Json) -> Result<Json, String> {
    let name = &def.name;
    let j = journal(def)?;
    let date = text_field(&request["date"], "the date")?.ok_or("a transaction needs a date")?;
    if !valid_date(date) {
        return Err(format!("{date} is not a date written as YYYY-MM-DD"));
    }
    let flag = text_field(&request["flag"], "the flag")?.unwrap_or("*");
    if flag != "*" && flag != "!" {
        return Err(format!("the flag is * or !, not {flag}"));
    }
    let kind = text_field(&request["kind"], "the kind")?.unwrap_or("original");
    if !GENERATED.contains(&kind) {
        return Err(format!(
            "the kind of a transaction is original, adjustment or reversal, not {kind}"
        ));
    }
    let narration = text_field(&request["narration"], "the narration")?.unwrap_or_default();
    if narration.contains(MARK_START) {
        return Err(format!(
            "the narration already says {MARK_START}...]: ClioTree adds that itself"
        ));
    }
    // Every transaction ClioTree writes says so, and what kind it is.
    let narration = &mark(narration, kind);
    let payee = text_field(&request["payee"], "the payee")?;
    let words = |key: &str, what: &str| -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        for w in request[key].as_array().into_iter().flatten() {
            let w = w
                .as_str()
                .unwrap_or_default()
                .trim_start_matches(['#', '^']);
            if !valid_word(w) {
                return Err(format!("{w:?} cannot be a {what}"));
            }
            out.push(w.to_owned());
        }
        Ok(out)
    };
    let tags = words("tags", "tag")?;
    let links = words("links", "link")?;
    let mut meta_lines = Vec::new();
    if let Some(meta) = request["meta"].as_object() {
        for (k, v) in meta {
            if !valid_key(k) {
                return Err(format!(
                    "{k} cannot be a metadata key: it starts with a small letter and has \
                     letters, digits, - and _"
                ));
            }
            let value = match v {
                Json::Object(o) if o.contains_key("$dec") => {
                    number(v, k)?.unwrap_or_default().to_string()
                }
                Json::Object(o) if o.contains_key("$date") => {
                    o["$date"].as_str().unwrap_or_default().to_owned()
                }
                Json::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_owned(),
                Json::Number(_) => number(v, k)?.unwrap_or_default().to_string(),
                _ => quoted(text_field(v, k)?.unwrap_or_default()),
            };
            meta_lines.push(format!("  {k}: {value}"));
        }
    }
    let Some(items) = request["postings"].as_array().filter(|p| p.len() >= 2) else {
        return Err("a transaction needs at least two postings".to_owned());
    };
    let mut postings = Vec::new();
    for (i, p) in items.iter().enumerate() {
        let what = format!("posting {}", i + 1);
        let account = text_field(&p["account"], &format!("the account of {what}"))?
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .ok_or_else(|| format!("{what} needs an account"))?;
        let amount = number(&p["amount"], &format!("the amount of {what}"))?;
        let currency =
            text_field(&p["currency"], &format!("the currency of {what}"))?.map(str::trim);
        if amount.is_some() != currency.is_some() {
            return Err(format!(
                "{what} needs both an amount and a currency, or neither"
            ));
        }
        if let Some(c) = currency
            && !valid_currency(c)
        {
            return Err(format!("{c} cannot be a currency"));
        }
        let cost = number(&p["cost"], &format!("the cost of {what}"))?;
        let cost_currency = text_field(&p["cost_currency"], "the cost currency")?.map(str::trim);
        let price = number(&p["price"], &format!("the price of {what}"))?;
        let price_currency = text_field(&p["price_currency"], "the price currency")?.map(str::trim);
        for (n, c, which) in [
            (cost, cost_currency, "cost"),
            (price, price_currency, "price"),
        ] {
            if n.is_some() != c.is_some() {
                return Err(format!(
                    "the {which} of {what} needs both a number and a currency"
                ));
            }
            if let Some(c) = c
                && !valid_currency(c)
            {
                return Err(format!("{c} cannot be a currency"));
            }
            if n.is_some() && amount.is_none() {
                return Err(format!("{what} has a {which} but no amount"));
            }
        }
        postings.push(Post {
            flag: None,
            account: account.to_owned(),
            amount,
            currency: currency.map(str::to_owned),
            cost,
            cost_currency: cost_currency.map(str::to_owned),
            cost_date: None,
            price,
            price_currency: price_currency.map(str::to_owned),
            meta: Vec::new(),
            auto: false,
        });
    }
    let mut txn = Txn {
        date: date.to_owned(),
        flag: flag.to_owned(),
        payee: payee.map(str::to_owned),
        narration: narration.to_owned(),
        tags: tags.clone(),
        links: links.clone(),
        meta: Vec::new(),
        file: String::new(),
        line: 0,
        postings: postings.clone(),
        padding: false,
    };
    complete(&mut txn).map_err(|why| format!("{name}: the transaction was not added: {why}"))?;
    for p in &postings {
        if let Some(why) = not_open(&j.accounts, &p.account, date) {
            return Err(format!("{name}: the transaction was not added: {why}"));
        }
        if let (Some(c), Some(info)) = (&p.currency, j.accounts.get(&p.account))
            && !info.currencies.is_empty()
            && !info.currencies.contains(c)
        {
            return Err(format!(
                "{name}: the transaction was not added: {} holds only {}, not {c}",
                p.account,
                info.currencies.join(", ")
            ));
        }
    }

    // The text, laid out as bean-format would.
    let mut head = format!("{date} {flag}");
    if let Some(payee) = payee {
        head.push(' ');
        head.push_str(&quoted(payee));
    }
    head.push(' ');
    head.push_str(&quoted(narration));
    for t in &tags {
        head.push_str(&format!(" #{t}"));
    }
    for l in &links {
        head.push_str(&format!(" ^{l}"));
    }
    let width = postings
        .iter()
        .map(|p| p.account.chars().count())
        .max()
        .unwrap_or(0)
        + 2;
    let mut lines = vec![head];
    lines.extend(meta_lines);
    for p in &postings {
        let mut line = format!("  {}", p.account);
        if let (Some(amount), Some(currency)) = (p.amount, &p.currency) {
            let pad = width - p.account.chars().count();
            line.push_str(&" ".repeat(pad));
            line.push_str(&format!("{amount} {currency}"));
            if let (Some(cost), Some(c)) = (p.cost, &p.cost_currency) {
                line.push_str(&format!(" {{{cost} {c}}}"));
            }
            if let (Some(price), Some(c)) = (p.price, &p.price_currency) {
                line.push_str(&format!(" @ {price} {c}"));
            }
        }
        lines.push(line);
    }
    let text = lines.join("\n") + "\n";

    let main = main_path(def)?;
    let _waiting = crate::sources::Waiting::begin();
    let existing = std::fs::read(&main).map_err(|e| format!("{}: {e}", main.display()))?;
    let mut prefix = String::new();
    if !existing.is_empty() && !existing.ends_with(b"\n") {
        prefix.push('\n');
    }
    if !existing.is_empty() {
        prefix.push('\n');
    }
    // The lines the file has before the transaction, then its own first line.
    let line = existing.iter().filter(|b| **b == b'\n').count() + prefix.matches('\n').count() + 1;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&main)
        .map_err(|e| format!("{} cannot be written: {e}", main.display()))?;
    file.write_all(format!("{prefix}{text}").as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|e| format!("{} cannot be written: {e}", main.display()))?;
    forget(&main);
    let shown = main
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(json!({ "file": shown, "line": line, "text": text }))
}

// ------------------------------------------------------------------ test

/// The Test: reads the journal and says what is in it.
pub fn test(def: &SourceDef) -> TestOutcome {
    let start = Instant::now();
    match journal(def) {
        Ok(j) => {
            let first = j.transactions.first().map(|t| t.date.clone());
            let last = j.transactions.last().map(|t| t.date.clone());
            let span = match (first, last) {
                (Some(a), Some(b)) => format!(", from {a} to {b}"),
                _ => String::new(),
            };
            let problems = match j.errors.len() {
                0 => String::new(),
                1 => format!("; 1 problem: {}", j.errors[0]),
                n => format!("; {n} problems, the first: {}", j.errors[0]),
            };
            TestOutcome {
                ok: true,
                message: format!(
                    "{} transactions in {} accounts{span}, {} {}{problems}, in {} ms",
                    j.transactions.iter().filter(|t| !t.padding).count(),
                    j.accounts.len(),
                    j.files,
                    if j.files == 1 { "file" } else { "files" },
                    start.elapsed().as_millis()
                ),
            }
        }
        Err(why) => TestOutcome::failed(why),
    }
}

#[cfg(test)]
#[path = "journal_tests.rs"]
pub(crate) mod journal_tests;
