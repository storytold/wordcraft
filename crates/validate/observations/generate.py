#!/usr/bin/env python3
"""Generate and audit the validator's observation contracts with a fail-closed Rust syntax recognizer.

The audit checks declared collection consumption, typed-accessor bypasses and register drift.
It deliberately does not certify the semantic truth of a declaration; fixtures do that.
"""
from pathlib import Path
import argparse, atexit, hashlib, json, os, re, shutil, subprocess, sys, tempfile, time
from contextlib import ExitStack
from functools import lru_cache, wraps
from collections import Counter

FIELDS = {
    'comment_defs','comment_refs','comment_para_ids','docpr','bookmarks','revisions',
    'footnote_refs','endnote_refs','footnote_defs','endnote_defs','header_refs','footer_refs','style_refs','num_ids',
    'listings','special_notes','content_parts','content_part_exemptions','ink_content_parts',
    'refs','entries','rels','content_types', 'note_listings',
}
BASE_FIELDS = frozenset(FIELDS)
ACCESSORS = {'prefix','is_complete','project','into_complete_set','missing','append','retain_budget','push_bounded','invalidate'}
# Input here is the recognizer's canonical Rust token stream, with literals masked.
# Non-ASCII token characters are identifier characters; Python's \w omits Rust XID
# continuations (combining marks) and some XID starts. Never split such an identifier.
IDENT_CHAR = r'[\w\x80-\U0010ffff]'
IDENT = IDENT_CHAR + '+'
IDENT_START = r'(?<![\w\x80-\U0010ffff])'
IDENT_END = r'(?![\w\x80-\U0010ffff])'
MEMBERSHIP = IDENT_START + r'((?:' + IDENT + r'\s*\.\s*)*' + IDENT + r')\s*\.\s*contains(?:_key)?\s*\('



def cache_large_text(function):
    cached = lru_cache(maxsize=256)(function)
    @wraps(function)
    def call(text):
        return cached(text) if len(text) >= 4096 else function(text)
    return call


@cache_large_text
def mask(text):
    """Blank Rust comments and literals, preserving all byte offsets and newlines."""
    pattern = r'//[^\n]*|/\*|r(#+)?"[\s\S]*?"\1|"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])\''
    # Raw strings need a backreference with an empty group rather than an unmatched optional one.
    pattern = pattern.replace('(#+)?', '(#*)')
    # Rust block comments nest. Stopping at the first */ leaves comment tokens
    # visible, where a semicolon or pipe could split a closure-parameter candidate.
    pieces = []; at = 0
    token = re.compile(pattern)
    while match := token.search(text, at):
        end = match.end()
        if match.group() == '/*':
            depth = 1
            end = len(text)
            for delimiter in re.finditer(r'/\*|\*/', text[match.end():]):
                depth += 1 if delimiter.group() == '/*' else -1
                if depth == 0:
                    end = match.end() + delimiter.end()
                    break
        pieces.extend((text[at:match.start()], blank(text[match.start():end])))
        at = end
    return ''.join(pieces) + text[at:]


def blank(text):
    """Preserve offsets/newlines without a regex replacement for every literal character."""
    if '\n' not in text:
        return ' ' * len(text)
    return '\n'.join(' ' * len(line) for line in text.split('\n'))


def close_brace(code, opening):
    depth = 1
    for match in re.finditer(r'[{}]', code[opening+1:]):
        depth += 1 if match.group() == '{' else -1
        if depth == 0:
            return opening+1+match.start()
    raise ValueError('unbalanced Rust braces in contract audit')


@cache_large_text
def production(text):
    code = mask(text)
    # Exclude inline test modules, but not #[cfg(test)] external mod declarations at the crate root.
    for match in list(re.finditer(r'#\[cfg\(test\)\]\s*(?:#\[[^\]]*\]\s*)*mod\s+\w+\s*\{', code)):
        end = close_brace(code, match.end()-1)+1
        text = text[:match.start()] + blank(text[match.start():end]) + text[end:]
    return text


def normalized(code):
    """Raw Rust identifiers have the same audit identity as ordinary identifiers."""
    return re.sub(r'\br#(?=[^\W\d])', '', code)


_RECOGNIZER = None
_TIMINGS = False


def file_sha256(path):
    digest=hashlib.sha256()
    with path.open('rb') as source:
        while block := source.read(1024*1024):
            digest.update(block)
    return digest.digest()


def recognizer_artifact(output, manifest):
    # Cargo owns the target/profile/platform layout. Never guess a dependency
    # artifact name, or fall back to a binary left by an earlier build.
    artifacts=set()
    for line in output.splitlines():
        item=json.loads(line)
        target=item.get('target', {})
        if (item.get('reason')=='compiler-artifact'
                and Path(item.get('manifest_path', '')).resolve()==manifest.resolve()
                and target.get('name')=='observation-recognizer'
                and target.get('kind')==['bin']
                and item.get('executable')):
            artifacts.add(Path(item['executable']))
    if len(artifacts)!=1:
        raise ValueError('Cargo did not report exactly one recognizer executable')
    return artifacts.pop()


def recognizer_binary(executable, directory):
    # Workers may republish Cargo's alias during process startup. Launch a
    # private copy instead, verified against a stable read of the fresh alias.
    destination=Path(directory)/executable.name
    deadline=time.monotonic()+2
    while True:
        try:
            expected=file_sha256(executable)
            shutil.copy2(executable, destination)
            if file_sha256(destination)==expected==file_sha256(executable):
                return destination
        except FileNotFoundError:
            pass
        if time.monotonic()>=deadline:
            raise ValueError('could not snapshot the freshly built recognizer executable')
        time.sleep(0.01)


def stop_recognizer(process, directory):
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
    directory.cleanup()


def recognize(sources):
    """Parse every audited token; an unsupported body or grammar node is an error.

    The helper is a standalone, locked build-time tool, never a validator dependency.
    It stays alive for in-memory self-tests so every mutation uses the real parser.
    """
    global _RECOGNIZER
    if _RECOGNIZER is None:
        root = Path(__file__).resolve().parents[3]
        manifest = Path(__file__).parent / 'recognizer/Cargo.toml'
        target = root / 'target/observation-recognizer'
        build_start=time.perf_counter()
        build=subprocess.run(['cargo', 'build', '--locked', '--quiet', '--manifest-path', str(manifest),
                              '--target-dir', str(target), '--message-format=json'],
                             check=True, stdout=subprocess.PIPE, text=True)
        if _TIMINGS: print(f'recognizer build: {time.perf_counter()-build_start:.3f}s', file=sys.stderr)
        executable=recognizer_artifact(build.stdout, manifest)
        directory=tempfile.TemporaryDirectory(prefix='wordcraft-observation-recognizer-')
        try:
            binary=recognizer_binary(executable, directory.name)
            if _TIMINGS: print(f'recognizer artifact: {executable}', file=sys.stderr)
            _RECOGNIZER = subprocess.Popen([str(binary)],
                                          stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        except BaseException:
            directory.cleanup()
            raise
        atexit.register(stop_recognizer, _RECOGNIZER, directory)
    _RECOGNIZER.stdin.write(json.dumps(sources) + '\n')
    _RECOGNIZER.stdin.flush()
    line = _RECOGNIZER.stdout.readline()
    if not line:
        raise ValueError('syntax recognizer terminated without accepting the sources')
    response = json.loads(line)
    if 'error' in response:
        raise ValueError(response['error'])
    return response['ok']


def consumption(code, enum_variants=(), semantic=None):
    if semantic:
        for scalar in semantic.get('scalar_reads', []):
            code = code.replace(scalar, '__scalar_budget')
        for path in sorted(set(semantic.get('scrub_paths', [])), key=len, reverse=True):
            code = code.replace(path, '__audited_accessor')
    # Qualified enum values are expressions, not collection types. Infer these
    # identities from recognized enum definitions instead of naming source exceptions.
    type_code = code
    for variant in enum_variants:
        if variant.rsplit('::', 1)[-1] not in {'Observed','CompleteSet','Vec','HashSet','HashMap','BTreeSet','BTreeMap'}:
            continue
        path = r'\s*::\s*'.join(map(re.escape, variant.split('::')))
        type_code = re.sub(IDENT_START + path + IDENT_END, '', type_code)
    fields = ['field.'+m.group(1) for m in re.finditer(r'\.\s*(' + IDENT + r')',code) if m.group(1) in FIELDS]
    methods = ['access.'+m.group(1) for m in re.finditer(r'\.\s*(' + IDENT + r')\s*(?:\(|::)',code) if m.group(1) in ACCESSORS]
    if semantic is not None and 'accessors' in semantic:
        methods = ['access.'+name for name in semantic['accessors']]
    parsers = ['parser.'+m.group(1) for m in re.finditer(IDENT_START + r'(parse_' + IDENT + r')\s*\(',code)]
    types = ['type.'+m.group() for m in re.finditer(IDENT_START + r'(?:Observed|CompleteSet|Vec|HashSet|HashMap|BTreeSet|BTreeMap)' + IDENT_END, type_code)]
    return [f'{name}*{count}' for name,count in sorted(Counter(fields + methods + parsers + types).items())]


def representation_footprint(header, code, semantic=None):
    private = Counter(re.findall(r'\bobserved_(?:value|status)\b', code))
    membership = Counter(re.sub(r'\s+', '', m.group(1)) for m in re.finditer(MEMBERSHIP, code))
    return sorted(consumption(header+code, semantic=semantic) + [f'repr.{k}*{v}' for k,v in sorted(private.items())]
                  + [f'membership.{k}*{v}' for k,v in sorted(membership.items())])


@cache_large_text
def discovered_fields(text):
    code=normalized(mask(production(text)))
    # The original regex is line-local. Cache unchanged lines across full-file
    # mutations, without retaining arbitrarily large line keys.
    return tuple(field for line in code.split('\n')
                 for field in (cached_field_line(line) if len(line)<=4096 else field_line(line)))


def field_line(line):
    return tuple(re.findall(r'^[ \t]*(?:pub(?:\(crate\))?[ \t]+)?(\w+)[ \t]*:[^\n]*(?:Observed|Vec|HashSet|HashMap|BTreeSet|BTreeMap)',line,re.M))


cached_field_line=lru_cache(maxsize=8192)(field_line)


def discover_fields(sources):
    for name,text in sources.items():
        if name in {"model.rs", "reserved.rs", "contracts_generated.rs"} or name.endswith("_tests.rs"): continue
        FIELDS.update(discovered_fields(text))



@lru_cache(maxsize=256)
def rule_registry(lib_text):
    return frozenset(re.findall(r'RuleSpec\s*\{\s*id:\s*"([A-Z0-9-]+)"', lib_text))


@lru_cache(maxsize=256)
def surface_text(name, items, lib_text):
    return normalized(mask(' '.join(items)))


# The semantic is cross-unit (notably helper/alias resolution), so raw body text
# alone is not a valid key. Policy text and the complete field/rule context also
# participate; cached records are only read by declarations().
@lru_cache(maxsize=4096)
def unit_audit(name, func, header, body, semantic_json, fields, enum_variants,
               known_rules, providers_text, entry_loss_text, provider_seen):
    errors=[]; rows=[]; seen=set(); seen_providers=set(); seen_entry_loss={}
    providers=json.loads(providers_text); entry_loss=json.loads(entry_loss_text)
    known_rules=frozenset(known_rules)
    def record():
        return (tuple(errors), tuple(rows), frozenset(seen), frozenset(seen_providers), tuple(seen_entry_loss.items()))
    header = normalized(mask(header)); code = normalized(mask(body))
    semantic = json.loads(semantic_json)
    errors.extend(f'{name}::{func}: {issue}' for issue in semantic.get('issues', []))
    state_predicates = semantic.get('state_predicates', [])
    state_key = f'{name}::{func}'
    if state_predicates:
        seen_entry_loss[state_key] = state_predicates
    if state_predicates != entry_loss.get(state_key, []):
        errors.append(f'{state_key}: entry-loss predicate differs from its named conservative/lookup contract')
    lookup_uses = [f'lookup.{k}*{v}' for k,v in sorted(Counter(semantic.get('lookups', [])).items())]
    uses = sorted(consumption(header+code, semantic=semantic) + lookup_uses)
    bounded_uses = consumption(header+code, enum_variants, semantic) + lookup_uses
    decl = re.search(r'observation_contract\s*!\s*\{', code)
    declaration = body[decl.end():close_brace(code, decl.end()-1)] if decl else ''
    if name != 'observed.rs' and re.search(r'\.\s*(?:observed_value|observed_status)\b',code):
        errors.append(f'{name}::{func}: bypasses private observation representation')
    if 'PackageRegistryContext' in header+code or re.search(r'(?:\.|::)\s*contexts\b',code):
        if (name,func) not in {('lib.rs','report_unreferenced_comments'),('lib.rs','check_custom_xml'),('ownership.rs','contexts')}:
            errors.append(f'{name}::{func}: undeclared package-registry exception')
    if re.search(r'\.\s*difference\s*\(', code):
        errors.append(f'{name}::{func}: raw absence difference bypasses CompleteSet')
    if name == 'observed.rs':
        # The private implementation has explicit build-time contracts too.
        # No function, initializer or consuming body can escape by file name.
        footprint = representation_footprint(header, code, semantic)
        if providers.get(func) != dict(header=header, collections=footprint) or provider_seen:
            errors.append(f'{name}::{func}: representation contract differs or is missing')
        seen_providers.add(func)
        rows.append(dict(helper=f'{name}::{func}', rules='-', collections=','.join(footprint) or '-',
            scope='private Observed/CompleteSet representation provider; signature and footprint pinned in observations/representation-contracts.json',
            loss='local completeness and allocation budgets carried by the typed representation',
            fallback='retain prefix; incomplete observations cannot construct CompleteSet; fixtures check semantics',
            package_loss='independent'))
        return record()
    # Raw definition membership must not bypass CompleteSet, even after copying a prefix.
    # These receivers are structural metadata, lexical validation, or allocation guards.
    structural_membership = {
        ('lib.rs', 'absence_suppressed', 'PACKAGE_ABSENCE_RULES'),
        ('lib.rs', 'check_note_pr', 'reverse_ok'),
        ('lib.rs', 'report_malformed', 'pkg.reachable'),
        ('lib.rs', 'check_typed_content', 'non_main_owner_rels'),
        ('lib.rs', 'report_unreferenced_comments', 'incomplete_owners'),
        ('lib.rs', 'supporting_part', 'acc.coverage'),
        ('lib.rs', 'check_custom_xml', 'custom_xml_owners'),
        ('lib.rs', 'check_media', 'image_targets'),
        ('scan.rs', 'check_protection', 'k'),
        ('scan.rs', 'check_para_ids', 'state.para_ids'),
        ('scan.rs', 'attr_plain', 'k'),
        ('package.rs', 'is_main_content_type', 'MAIN_CONTENT_TYPES'),
        ('package.rs', 'load', 'xml_parts'),
        ('package.rs', 'classified_xml_parts', 'role_targets'),
        ('package.rs', 'is_xml_part', 'pkg.xml_parts'),
        ('ownership.rs', 'with_budget', 'roots'),
        ('ownership.rs', 'parts', 'owners'),
    }
    for access in re.finditer(MEMBERSHIP, code):
        receiver=re.sub(r'\s+', '', access.group(1))
        if (name,func,receiver) not in structural_membership and not (decl and receiver in semantic.get('positive_membership', [])):
            errors.append(f'{name}::{func}: raw membership {receiver} bypasses CompleteSet or undeclared structural scope')
    if not bounded_uses and not decl and not re.search(r'\bacc\s*\.\s*(?:add|add_graded|ran)\s*\(', code):
        return record()
    if not decl:
        errors.append(f'{name}::{func}: bounded consumption has no observation contract: {uses}')
        return record()
    data=dict(re.findall(r'(\w+)\s*:\s*"((?:\\.|[^"\\])*)"\s*;',declaration))
    required={'rules','collections','scope','loss','fallback','package_loss'}
    if set(data)!=required or any(not data.get(k,'').strip() for k in required):
        errors.append(f'{name}::{func}: missing contract field/fallback')
        return record()
    declared=[] if data['collections']=='-' else sorted(data['collections'].split(','))
    if declared!=uses:
        errors.append(f'{name}::{func}: declared collections differ from bounded accesses: declared={declared}, used={uses}')
    declared_rules=set(data['rules'].split(',')) - {'-'}
    called_rules=set(re.findall(r'"([A-Z0-9-]+)"',body)) & known_rules
    if not called_rules <= declared_rules:
        errors.append(f'{name}::{func}: rule consumption omitted: {called_rules-declared_rules}')
    if not declared_rules <= known_rules:
        errors.append(f'{name}::{func}: unknown rule IDs: {declared_rules-known_rules}')
    if data['package_loss'] != 'independent' and not set(data['package_loss'].split(',')) <= declared_rules:
        errors.append(f'{name}::{func}: undeclared unknown-package-identity rule policy')
    key = (func, header, code, tuple(sorted(data.items())))
    if key not in seen:
        rows.append(dict(helper=f'{name}::{func}', **data))
        seen.add(key)
    return record()


def declarations(sources):
    rows=[];errors=[]
    providers_text = (Path(__file__).parent / 'representation-contracts.json').read_text()
    providers = json.loads(providers_text)
    seen_providers = set()
    entry_loss_text = (Path(__file__).parent / 'entry-loss-contracts.json').read_text()
    entry_loss = json.loads(entry_loss_text)
    seen_entry_loss = {}
    FIELDS.clear(); FIELDS.update(BASE_FIELDS)
    discover_fields(sources)
    known_rules=rule_registry(sources['lib.rs'])
    audited = {name: text for name, text in sources.items()
               if not name.endswith('_tests.rs') and name not in {'model.rs','reserved.rs','contracts_generated.rs'}}
    recognized = recognize(audited)
    reader_contracts = json.loads((Path(__file__).parent / 'reader-contracts.json').read_text())
    allowed_readers = Counter(json.dumps(r, sort_keys=True) for r in reader_contracts['readers'])
    actual_readers = Counter(json.dumps(r, sort_keys=True) for f in recognized.values() for r in f['readers'])
    for record in actual_readers - allowed_readers:
        reader = json.loads(record)
        errors.append(f"{reader['type']}: unauthorized reader derive {reader['derive']} or changed representation/DTO field shape")
    FIELDS.update(f for file in recognized.values() for f in file['collection_fields'])
    enum_variants = [v for file in recognized.values() for v in file['enum_variants']]
    field_key=frozenset(FIELDS); variant_key=tuple(enum_variants); rule_key=tuple(sorted(known_rules))
    for name,text in sorted(sources.items()):
        if name.endswith('_tests.rs') or name in {'model.rs','reserved.rs','contracts_generated.rs'}:
            continue
        for module in recognized[name]['modules']:
            if module + '.rs' not in sources:
                errors.append(f'{name}: external module {module} has no supplied audited source or explicit excluded table/test/policy source')
        surface = surface_text(name, tuple(recognized[name]['surface']), sources['lib.rs'])
        if re.search(r'\bPackageRegistryContext\b|(?:\.|::)\s*contexts\b', surface):
            errors.append(f'{name}: module-scope package-registry aliases/queries are unsupported')
        # A template and an invocation may yield identical units. Audit all instances;
        # emit a single row only when the entire normalized header/body is identical.
        seen = set()
        for unit in recognized[name]['units']:
            func, header, body = unit['name'], unit['header'], unit['body']
            record = unit_audit(name, func, header, body,
                json.dumps(unit.get('semantic', {}), sort_keys=True, separators=(',', ':')),
                field_key, variant_key, rule_key,
                providers_text, entry_loss_text, func in seen_providers)
            unit_errors, unit_rows, keys, providers_seen, entry_seen = record
            errors.extend(unit_errors)
            seen_providers.update(providers_seen)
            seen_entry_loss.update(entry_seen)
            if not keys or not keys <= seen:
                rows.extend(unit_rows)
                seen.update(keys)
    if seen_entry_loss != entry_loss: errors.append('entry-loss producer/conservative-reader contracts missing')
    if seen_providers != set(providers): errors.append('representation contracts missing providers')
    covered={rule for row in rows for rule in row['rules'].split(',')}
    if known_rules-covered: errors.append(f'rule registry lacks observation contracts: {sorted(known_rules-covered)}')
    if errors:
        raise ValueError('\n'.join(errors))
    return rows


def outputs(rows):
    header = ('# Validator observation contracts\n\n'
        'Generated by `python3 crates/validate/observations/generate.py`; do not edit.\n'
        'Run with `--check` to audit consumers and drift; `--self-test` exercises rejection paths.\n\n'
        'The gate enforces fail-closed syntax recognition over the production Rust in `src/lib.rs`, `src/observed.rs`, `src/ownership.rs`, `src/package.rs`, `src/resolution.rs`, and `src/scan.rs`. Inline `#[cfg(test)]` modules, separate test files, generated policy, and the non-consuming `model.rs`/`reserved.rs` tables are outside this boundary.\n'
        'Every item, expression body and pattern in those files must match the closed grammar in `observations/recognizer/src/main.rs`; unsupported syntax fails. External module declarations must resolve to a supplied source file or an explicit table/test/policy exclusion. Functions and impl methods, nested closures, const/static initializers, discriminants and type-level expressions are audited. Named-field patterns in binding, parameter, closure, match and assignment positions, pattern macros, traits and unaudited item/body kinds are rejected. Raw identifiers are normalized before footprint and registry checks.\n'
        'Single-arm item macro templates with ident/literal fragments and no repetition are instantiated and audited, both at definition and invocation; all other definitions and item macros fail. `observation_contract!` must expand to no code. The recognized `format!`, `vec!`, `matches!` and `debug_assert!` argument grammars (unqualified or canonical `std::` paths only) are parsed and traversed, including their patterns and nested bodies; other expression macros and local definitions/imports shadowing these standard macros and glob imports fail. Named collection fields are discovered from recognized expansions too. Footprint identifiers retain complete Unicode identifiers, including combining marks; Python word boundaries are not Rust identifier boundaries.\n'
        'Exact declared lexical footprints cover structural adapters and collection-typed locals; enum values alone do not trigger a collection declaration. Private representation providers have explicit signature/footprint contracts in `observations/representation-contracts.json`, with no file-wide function exemption. Macro instances with identical normalized bodies/headers/contracts share a register row. Package helper imports, module aliases and local function bindings are resolved in the closed grammar. Entry lookups carry Resolution; absence requires an explicit Absent arm, with Unknown separate. Literal Absent construction is sealed to resolution.rs, discovered lookups must consult both entry-loss states, and producer/conservative state predicates are pinned in observations/entry-loss-contracts.json. Raw iterator/map absence cannot be laundered by a contract. Value-level polarity decisions are not audited by the gate: equality against a `Resolution` variant (S1/I7), and branching on its `Debug` rendering (the type has `Debug`, not `Display`), are accepted even when the branch is inverted. The gate still traverses arm bodies syntactically and rejects unsupported shapes; the exception is the body\'s truth value, not the body\'s syntax. The suite catches these forms only where a wired site is covered by a test; an uncalled helper (S10) changes no verdict. Fixtures check semantic truth; this is not general Rust name resolution or a whole-program dataflow proof.\n'
        'Reader derives requested in audited files are enforced by canonical identity (Debug, serde::Serialize and serde::Deserialize), including qualified and renamed imports. Exact type+derive+field/generic shapes, including the existing package/registry representation readers, are recorded in `observations/reader-contracts.json`; new pairs and added registry fields fail closed. The existing DTO pairs and representation pairs are listed below. Build scripts, compiler implementations and expansions not requested by these audited files remain outside this boundary. Other attribute macros and `cfg_attr` fail. The closed inert codegen list is `inline`, `inline(always)`, `inline(never)`, and `cold`; cfg and derives retain their separate traversed/identity-checked treatment. Serde field attributes are not inert. Named-field bindings and trait items/default bodies remain rejected. Rust privacy seals the observation representation. The standalone locked recognizer is build-time tooling and does not enter validator/WASM dependencies.\n'
        'Only `check_custom_xml` (CUSTOMXML-SCHEMA) and `report_unreferenced_comments` (informational coverage) may consult PackageRegistryContext.\n'
        'That context admits established-unowned declarations to the primary package registry; it never grants story ownership.\n'
        'Duplicate relationship IDs bind the first XML declaration in every reference path; swapping duplicates changes binding evidence.\n\n'
        '| Helper | Rules/conclusion | Collections/accessors | Intentional scope | Incompleteness causes | Fallback | Unknown package identity |\n'
        '|---|---|---|---|---|---|---|\n')
    policy = json.loads((Path(__file__).parent / 'reader-contracts.json').read_text())
    pairs = sorted({r['type']+' + '+r['derive'] for r in policy['readers']})
    marker = '| Helper | Rules/conclusion |'
    at = header.index(marker)
    header = header[:at] + 'Recorded reader pairs (exact shapes in the policy):\n\n' + ''.join('- `'+pair+'`\n' for pair in pairs) + '\n' + header[at:]
    md=header+''.join('| '+' | '.join(row[k].replace('|','/') for k in ['helper','rules','collections','scope','loss','fallback','package_loss'])+' |\n' for row in rows)
    rules=sorted({r for row in rows if row['package_loss']!='independent' for r in row['package_loss'].split(',')})
    rust='//! Generated observation-contract policy. Do not edit; see observations/generate.py.\n'
    rust+='/// Rules whose failing absence conclusions cannot survive an unknown package identity.\n'
    rust+='pub(crate) const PACKAGE_ABSENCE_RULES: &[&str] = &[\n'+''.join(f'    "{r}",\n' for r in rules)+'];\n'
    return md,rust


def mutation_cases():
    # Mutate in memory only. Neither repository source nor generated output is rewritten.
    mutations = {
      'undeclared consumer': ('lib.rs', lambda s:s+'\nfn invented_check(value: Observed<Vec<u8>>) { let _ = value.prefix(); }\n'),
      'undeclared array consumer': ('lib.rs', lambda s:s+'\nfn invented_array(v: &[Observed<Vec<u8>>; 2]) { let _ = v[0].prefix(); }\n'),
      'missing fallback': ('lib.rs', lambda s:s.replace('fallback: \"', 'removed_fallback: \"',1)),
      'accessor bypass': ('lib.rs', lambda s:s+'\nfn invented_bypass(value: Observed<Vec<u8>>) { let _ = value.observed_value; }\n'),
      'exception leakage': ('lib.rs', lambda s:s+'\nfn invented_registry() { let _ = ownership::PackageRegistryContext::new(graph); }\n'),
      'new undeclared accessor': ('lib.rs',lambda s:s.replace('acc.ran("CUSTOMXML-SCHEMA");','acc.ran("CUSTOMXML-SCHEMA"); let _ = new_collection.prefix();',1)),
      'raw membership bypass': ('lib.rs',lambda s:s.replace('acc.ran("CUSTOMXML-SCHEMA");','acc.ran("CUSTOMXML-SCHEMA"); let absent = !raw.contains(&1);',1)),
      'raw absence difference': ('lib.rs',lambda s:s+'\nfn invented_absence() { let _ = a.difference(b); }\n'),
      'destructured package read': ('lib.rs',lambda s:s+'\nfn unregistered_read(pkg: &Package) -> usize { let Package { entries: items, .. } = pkg; items.len() }\n'),
      'module registry alias and method pointer': ('lib.rs',lambda s:s+'''\nuse ownership::PackageRegistryContext as HiddenRegistry;
const REGISTRY_QUERY: for<'a> fn(&'a HiddenRegistry, &str) -> &'a [OwnerId] = HiddenRegistry::contexts;
fn unregistered_registry(graph: &PartGraph) -> bool {
    let registry = HiddenRegistry::new(graph);
    REGISTRY_QUERY(&registry, "x").is_empty()
}
'''),
      'destructured parameter read': ('lib.rs',lambda s:s+'\nfn unregistered_parameter(Package { entries: items, .. }: &Package) -> usize { items.len() }\n'),
      'destructured match read': ('lib.rs',lambda s:s+'\nfn unregistered_match(pkg: &Package) -> usize { match pkg { Package { entries: items, .. } => items.len() } }\n'),
      'destructured for read': ('lib.rs',lambda s:s+'\nfn unregistered_for(packages: &[Package]) { for Package { entries: items, .. } in packages { use_items(items); } }\n'),
      'immediately invoked destructured closure': ('lib.rs',lambda s:s+'\nfn unregistered_read(pkg: &Package) -> usize { (|Package { entries: items, .. }: &Package| items.len())(pkg) }\n'),
      'let-bound destructured closure': ('lib.rs',lambda s:s+'\nfn unregistered_read(pkg: &Package) -> usize { let count = |Package { entries: items, .. }: &Package| items.len(); count(pkg) }\n'),
      'const fn-pointer destructured closure': ('lib.rs',lambda s:s+'\nconst UNREGISTERED_READ: fn(&Package) -> usize = |Package { entries: items, .. }| items.len();\n'),
      'closure argument with nested pattern': ('lib.rs',lambda s:s+'\nfn unregistered_read() { consume(move |(_, [Package { entries: items, .. }]): ((), [&Package; 1])| items.len()); }\n'),
      'returned closure with later parameter': ('lib.rs',lambda s:s+'\nfn unregistered_read() -> impl Fn((), &Package) -> usize { move |_, &Package { ref entries, .. }| entries.len() }\n'),
      'static fn-pointer destructured closure': ('lib.rs',lambda s:s+'\nstatic UNREGISTERED_READ: fn(&Package) -> usize = |crate::package::Package { entries: items, .. }| items.len();\n'),
      'array-element destructured closure': ('lib.rs',lambda s:s+'\nfn unregistered_read() { consume([|whole @ Package { entries: _, .. }: &Package| use_items(whole)]); }\n'),
      'async destructured closure': ('lib.rs',lambda s:s+'\nfn unregistered_read() { consume(async move |Package { entries, .. }: &Package| entries.len()); }\n'),
      'nested struct closure pattern': ('lib.rs',lambda s:s+'\nfn unregistered_read() { consume(|Wrapper { inner: Package { entries: items, .. } }| items.len()); }\n'),
      'closure pattern after associated-type constraint': ('lib.rs',lambda s:s+'\nfn unregistered_read() { consume(|_: &dyn Trait<Item = ()>, Package { entries, .. }: &Package| entries.len()); }\n'),
      'closure pattern with attributes and comments': ('lib.rs',lambda s:s+'\nfn unregistered_read() { consume(|#[allow(unused)] /* pattern */ (Package { entries, .. },): (&Package,)| entries.len()); }\n'),
      'destructured closure returning adjacent closure': ('lib.rs',lambda s:s+'\nfn unregistered_read() { consume(|Package { entries, .. }: &Package||()| entries.len()); }\n'),
      'adjacent inner destructured closure': ('lib.rs',lambda s:s+'\nfn unregistered_read() { consume(|()||Package { entries, .. }: &Package| entries.len()); }\n'),
      'closure with parenthesized or-pattern': ('lib.rs',lambda s:s+'\nfn unregistered_read() { consume(|(Package { entries, .. } | Package { entries, .. })| entries.len()); }\n'),
      'closure with generic struct path': ('lib.rs',lambda s:s+'\nfn unregistered_read() { consume(|Package::<Vec<u8>> { entries, .. }| entries.len()); }\n'),
      'closure with qualified struct path': ('lib.rs',lambda s:s+'\nfn unregistered_read() { consume(|<Container as Trait>::Package { entries, .. }| entries.len()); }\n'),
      'closure with Unicode identifier continuation': ('lib.rs',lambda s:s+'\ntype Package\u0301 = Package;\nfn unregistered_read() { consume(|Package\u0301 { entries, .. }| entries.len()); }\n'),
      'closure with nested block comment': ('lib.rs',lambda s:s+'\nfn unregistered_read() { consume(|Package /* outer /* inner */ ; */ { entries, .. }| entries.len()); }\n'),
      'module registry type alias': ('lib.rs',lambda s:s+'\ntype HiddenRegistry = ownership::PackageRegistryContext;\n'),
      'module registry method pointer': ('lib.rs',lambda s:s+"\nconst QUERY: for<'a> fn(&'a ownership::PackageRegistryContext, &str) -> &'a [OwnerId] = ownership::PackageRegistryContext::contexts;\n"),
    }
    mutations.update({
      'delta16 destructuring assignment': ('lib.rs', lambda s:s+'\nfn unregistered_read(pkg: Package) -> usize { let items; Package { entries: items, .. } = pkg; items.len() }'),
      'delta16 parenthesized destructuring assignment': ('lib.rs', lambda s:s+'\nfn unregistered_read(pkg: Package) -> usize { let items; (Package { entries: items, .. }) = pkg; items.len() }'),
      'delta16 macro-produced closure pattern': ('lib.rs', lambda s:s+'''
macro_rules! package_pattern { ($items:ident) => { Package { entries: $items, .. } }; }
fn unregistered_read(pkg: &Package) -> usize { (|package_pattern!(items): &Package| items.len())(pkg) }
'''),
      'delta16 raw field read': ('lib.rs', lambda s:s+'\nfn unregistered_read(pkg: &Package) -> usize { pkg.r#entries.len() }'),
      'delta16 raw registry method': ('ownership.rs', lambda s:s.replace('impl PackageRegistryContext {', 'impl PackageRegistryContext { fn unregistered_registry(&self) -> usize { self.r#contexts("x").len() }', 1)),
      'delta16 module callback body': ('lib.rs', lambda s:s+'\nconst UNREGISTERED_READ: fn(&Package) -> usize = |pkg| pkg.entries.len();'),
      'static callback body': ('lib.rs', lambda s:s+'\nstatic UNREGISTERED_READ: fn(&Package) -> usize = |pkg| (|p: &Package| p.entries.len())(pkg);'),
      'raw private representation': ('lib.rs', lambda s:s+'\nfn invented(value: u8) { let _ = value.r#observed_value; }'),
      'raw accessor': ('lib.rs', lambda s:s+'\nfn invented(value: u8) { let _ = value.r#prefix(); }'),
      'raw registry type alias': ('lib.rs', lambda s:s+'\ntype Hidden = ownership::r#PackageRegistryContext;'),
      'raw registry function name': ('ownership.rs', lambda s:s.replace('impl PackageRegistryContext {', 'impl PackageRegistryContext { fn r#unregistered_registry(&self) -> usize { self.contexts("x").len() }', 1)),
      'registry in representation provider': ('observed.rs', lambda s:s+'\nfn hidden() { let _ = x.r#contexts("x"); }'),
      'unsupported union item': ('lib.rs', lambda s:s+'\nunion Unsupported { field: usize }'),
      'unsupported trait body': ('lib.rs', lambda s:s+'\ntrait Hidden { fn read(pkg: &Package) -> usize { pkg.entries.len() } }'),
      'unsupported foreign item': ('lib.rs', lambda s:s+'\nunsafe extern "C" { fn hidden(); }'),
      'unsupported production module': ('lib.rs', lambda s:s+'\nmod hidden { fn read(pkg: &super::Package) -> usize { pkg.entries.len() } }'),
      'unsupported attribute macro': ('lib.rs', lambda s:s+'\n#[hidden] fn read() {}'),
      'unsupported pattern macro in binding': ('lib.rs', lambda s:s+'\nfn read(pkg: &Package) { let hidden!(items) = pkg; }'),
      'unsupported pattern macro in parameter': ('lib.rs', lambda s:s+'\nfn read(hidden!(items): &Package) {}'),
      'unsupported pattern macro in match': ('lib.rs', lambda s:s+'\nfn read(pkg: &Package) { match pkg { hidden!(items) => {} } }'),
      'unsupported pattern macro in assignment': ('lib.rs', lambda s:s+'\nfn read(pkg: Package) { hidden!(items) = pkg; }'),
      'unsupported expression macro': ('lib.rs', lambda s:s+'\nfn read(pkg: &Package) { hidden!(pkg.entries); }'),
      'unsupported macro repetition': ('lib.rs', lambda s:s+'\nmacro_rules! hidden { ($($x:ident),*) => { $($x)* }; }'),
      'unsupported async body': ('lib.rs', lambda s:s+'\nfn read(pkg: &Package) { let _ = async { pkg.entries.len() }; }'),
      'nested closure body': ('lib.rs', lambda s:s+'\nfn read(pkg: &Package) { consume(|| (|p: &Package| p.entries.len())(pkg)); }'),
      'impl callback initializer': ('lib.rs', lambda s:s+'\nimpl Package { const READ: fn(&Package) -> usize = |pkg| pkg.entries.len(); }'),
      'enum discriminant body': ('lib.rs', lambda s:s+'\nenum Hidden { Value = (|pkg: &Package| pkg.entries.len())(PKG) }'),
      'type-level constant body': ('lib.rs', lambda s:s+'\ntype Hidden = [u8; (|pkg: &Package| pkg.entries.len())(PKG)];'),
      'macro-generated consumer': ('lib.rs', lambda s:s+'\nmacro_rules! hidden { ($f:ident) => { fn $f(pkg: &Package) -> usize { pkg.entries.len() } }; } hidden!(read);'),
      'macro-captured collection field': ('lib.rs', lambda s:s+'\nmacro_rules! hidden { ($f:ident) => { struct Hidden { $f: Vec<u8> } fn read(p: &Hidden) -> usize { p.$f.len() } }; } hidden!(brand_new);'),
      'contract macro starts emitting code': ('lib.rs', lambda s:s.replace('package_loss:literal;) => {};', 'package_loss:literal;) => { let _ = pkg.entries; };', 1)),
    })
    mutations.update({
      'contract text in literal is not a declaration': ('lib.rs', lambda s:s+'\nfn invented(pkg: &Package) { let text = r#"observation_contract! { rules: \"-\"; collections: \"field.entries*1\"; scope: \"x\"; loss: \"x\"; fallback: \"x\"; package_loss: \"independent\"; }"#; pkg.entries.len(); }'),
      'standard macro shadow hides consuming expansion': ('lib.rs', lambda s:s+'\nmacro_rules! format { ($field:ident) => { fn hidden(pkg: &Package) -> usize { pkg.$field.len() } }; } fn outer(pkg: &Package) -> usize { format!(entries); hidden(pkg) }'),
      'standard macro import shadow': ('lib.rs', lambda s:s+'\nuse external::format;'),
      'standard macro alias shadow': ('lib.rs', lambda s:s+'\nuse external::hidden as r#matches;'),
      'glob import could hide standard macro shadow': ('lib.rs', lambda s:s+'\nuse external::*;'),
      'attribute macro on item invocation': ('scan.rs', lambda s:s.replace('numeric_id!(DecimalId, 10);', '#[hidden] numeric_id!(DecimalId, 10);', 1)),
      'attribute macro on definition': ('scan.rs', lambda s:s.replace('macro_rules! numeric_id', '#[hidden] macro_rules! numeric_id', 1)),
      'external module without supplied source': ('lib.rs', lambda s:s+'\nmod hidden;'),
      'Unicode combining collection field': ('lib.rs', lambda s:s+'\nstruct UnicodeCollection { fresh\u0301: Vec<u8> } fn hidden(p: &UnicodeCollection) -> usize { p.fresh\u0301.len() }'),
      'Unicode combining membership receiver': ('lib.rs', lambda s:s+'\nfn hidden(fresh\u0301: &[u8]) -> bool { fresh\u0301.contains(&1) }'),
      'Unicode combining parser identifier': ('lib.rs', lambda s:s+'\nfn parse_fresh\u0301() -> usize { 1 } fn hidden() -> usize { parse_fresh\u0301() }'),
      'undeclared representation consumer': ('observed.rs', lambda s:s+'\nfn read(pkg: &Package) -> usize { pkg.entries.len() }'),
      'representation membership bypass': ('observed.rs', lambda s:s.replace('!self.0.contains(key)', '!self.0.contains(key) || raw.contains(key)', 1)),
    })
    mutations.update({
      'R18 obsolete Option absence': ('lib.rs', lambda s: re.sub(r'if match package::canonical_entry\(pkg, target\) \{.*?\n            \} \{', 'if package::canonical_entry(pkg, target).is_none() {', s, count=1, flags=re.S)),
      'R18 raw iterator absence': ('package.rs', lambda s: s+'\nfn unregistered_absence(entries: &[Entry], name: &str) -> bool { !entries.iter().any(|entry| entry.name == name) }'),
      'R18 contracted raw iterator absence': ('package.rs', lambda s: s+\
          '\nfn unregistered_absence(entries: &[Entry], name: &str) -> bool { observation_contract! { rules: "-"; collections: "field.entries*1"; scope: "x"; loss: "x"; fallback: "x"; package_loss: "independent"; } !entries.iter().any(|entry| entry.name == name) }'),
      'R18 helper-mediated absence': ('lib.rs', lambda s: s+'\nfn unregistered_absence(pkg: &Package, name: &str) -> bool { entry_by_name(pkg, name).is_none() }'),
      'R18 raw slice alias': ('package.rs', lambda s: s+'\ntype HiddenEntries = [Entry]; fn unregistered_absence(entries: &HiddenEntries, name: &str) -> bool { !entries.iter().any(|entry| entry.name == name) }'),
      'R18 fabricated absence': ('package.rs', lambda s: s+'\nfn invented() -> Resolution<()> { Resolution::Absent }'),
      'R18 fabricated completeness': ('package.rs', lambda s: s.replace('Resolution::discovered(canonical(&pkg.entries, name), !pkg.entries_truncated && !pkg.identity_incomplete)', 'Resolution::discovered(canonical(&pkg.entries, name), true)', 1)),
      'R18 Unknown-or-Absent': ('lib.rs', lambda s: s.replace('Resolution::Absent => true,\n                Resolution::Unknown(_) => false,', 'Resolution::Absent | Resolution::Unknown(_) => true,', 1)),
      'R18 missing Unknown': ('lib.rs', lambda s: s.replace('                Resolution::Unknown(_) => false,\n', '', 1)),
      'R18 direct state deletion': ('lib.rs', lambda s: s.replace('pkg.entries_truncated || pkg.identity_incomplete', 'pkg.identity_incomplete', 1)),
      'R18 direct state negation': ('lib.rs', lambda s: s.replace('pkg.entries_truncated || pkg.identity_incomplete', '!pkg.entries_truncated || pkg.identity_incomplete', 1)),
      'R18 discovery complete deletion': ('lib.rs', lambda s: s.replace('!pkg.entries_truncated\n        && !pkg.identity_incomplete', '!pkg.identity_incomplete', 1)),
      'R18 discovery complete negation': ('lib.rs', lambda s: s.replace('!pkg.entries_truncated\n        && !pkg.identity_incomplete', 'pkg.entries_truncated\n        && !pkg.identity_incomplete', 1)),
      'R18 reader assembly limit inversion': ('package.rs', lambda s: s.replace('zip.len() > limits.entries', 'zip.len() < limits.entries', 1)),
      'R18 canonical external call': ('package.rs', lambda s: s+'\nfn invented(entries: &[Entry], name: &str) -> bool { canonical(entries, name).is_none() }'),
    })
    for helper in ['canonical_entry', 'entry_by_name', 'content_unavailable']:
        for binding in ['hidden_lookup', 'package::'+helper]:
            mutations['R18 helper alias/value '+helper+'/'+binding] = ('lib.rs', lambda s, h=helper, b=binding: s+f'\nuse package::{h} as hidden_lookup; fn unregistered_absence(pkg: &Package, name: &str) -> bool {{ let lookup = {b}; let _ = lookup(pkg, name); false }}')
    for source in ['Self::prefix(self)', '<Self>::prefix(self)', '<Observed<HashSet<String>>>::prefix(self)']:
        for operation in ['!ACCESS.contains(key)', 'ACCESS.contains(key) == false', '!ACCESS.iter().any(|candidate| candidate == key)', 'matches!(ACCESS.contains(key), false)', 'match ACCESS.contains(key) { false => true, true => false }']:
            expression = operation.replace('ACCESS', source)
            mutations['R18 retained prefix '+expression] = ('lib.rs', lambda s, e=expression: s+'\nimpl Observed<HashSet<String>> { fn unregistered_absence(&self, key: &String) -> bool { '+e+' } }')
    for callable in ['Self::prefix', 'HiddenDefinitions::prefix']:
        mutations['R18 accessor function binding '+callable] = ('lib.rs', lambda s, c=callable: s+'\ntype HiddenDefinitions = Observed<HashSet<String>>; impl Observed<HashSet<String>> { fn unregistered_absence(&self, key: &String) -> bool { let prefix = '+c+'; !prefix(self).contains(key) } }')
    mutations['R18 aliased UFCS'] = ('lib.rs', lambda s: s+'\ntype HiddenDefinitions = Observed<HashSet<String>>; fn unregistered_absence(value: &HiddenDefinitions, key: &String) -> bool { !HiddenDefinitions::prefix(value).contains(key) }')
    mutations['R18 local prefix with exact original contract'] = ('lib.rs', lambda s: s.replace('self.parts.values().flat_map(|p| p.special_notes.prefix().iter().cloned()).collect()', 'self.parts.values().flat_map(|p| { let retained = p.special_notes.prefix(); retained.iter().filter(|candidate| !retained.contains(candidate)).cloned() }).collect()', 1))
    mutations['R18 retained map get absence'] = ('lib.rs', lambda s: s+'\nimpl Observed<HashMap<String, String>> { fn unregistered_absence(&self, key: &String) -> bool { Self::prefix(self).get(key).is_none() } }')
    for spelling in ['serde::Serialize', 'serde::Deserialize', 'Debug']:
        mutations['R18 unauthorized ContentTypes reader '+spelling] = ('package.rs', lambda s, d=spelling: s.replace('#[derive(Default)]\npub(crate) struct ContentTypes', '#[derive(Default, '+d+')]\npub(crate) struct ContentTypes', 1))
    mutations['R18 renamed reader derive'] = ('package.rs', lambda s: s.replace('#[derive(Default)]\npub(crate) struct ContentTypes', '#[derive(Default, RegistrySerialize)]\npub(crate) struct ContentTypes', 1)+'\nuse serde::Serialize as RegistrySerialize;\n')
    mutations['R18 unauthorized reader on DTO'] = ('lib.rs', lambda s: s.replace('pub struct Finding {', '#[derive(serde::Deserialize)] pub struct Finding {', 1))
    mutations['R18 registry fields added to allowed DTO'] = ('lib.rs', lambda s: s.replace('pub struct Finding {', 'pub struct Finding { hidden_registry: package::ContentTypes,', 1))
    def nested_reader(source):
        definition = re.search(r'(pub\(crate\) struct Limits \{.*?\n\})', source, re.S)
        if not definition: raise ValueError('nested reader fixture did not find Limits')
        source = source.replace('#[derive(Debug, Clone, Copy)]\npub(crate) struct Limits', '#[derive(Clone, Copy)]\npub(crate) struct Limits', 1)
        return source+'\nfn unregistered_nested_reader() { #[derive(Debug)] '+definition[1]+' }'
    mutations['R18 reader allowance cannot transfer into a function scope'] = ('lib.rs', nested_reader)
    mutations['R18 same-named DTO in another module'] = ('package.rs', lambda s: s+'\n#[derive(Debug)] struct Finding { rule: String }')
    mutations['R18 unsupported qualified env macro'] = ('lib.rs', lambda s: s+'\nfn unregistered() { std::env!("X"); }')
    # Amendment 1's sole advance diagnostic change: audit the supported path's arguments.
    mutations['seam/other-macro-path (R18 argument footprint)'] = ('lib.rs', lambda s: s+'\nfn unregistered_read(pkg: &Package) -> usize { std::format!("{}", pkg.entries.len()); pkg.entries.len() }')
    mutations['R18 qualified consuming macro arguments'] = ('lib.rs', lambda s: s+'\nfn unregistered_read(pkg: &Package) -> String { std::format!("{}", pkg.entries.len()) }')
    mutations['R18 standard namespace import shadow'] = ('lib.rs', lambda s: s+'\nuse external as std;')
    mutations['R18 derive outside type context'] = ('lib.rs', lambda s: s+'\n#[derive(Debug)] fn unregistered() {}')
    mutations['R18 serde field attribute outside policy'] = ('lib.rs', lambda s: s+'\n#[serde(skip)] fn unregistered() {}')
    mutations['R18 inline outside closed options'] = ('lib.rs', lambda s: s+'\n#[inline(sometimes)] fn unregistered() {}')
    getter_contract = 'observation_contract! { rules: "-"; collections: "access.prefix*1"; scope: "prefix presence control"; loss: "incomplete observed map"; fallback: "only presence is established"; package_loss: "independent"; }'
    mutations['R18 contracted retained get equality absence'] = ('lib.rs', lambda s: s+'\nimpl Observed<HashMap<String, String>> { fn unregistered_absence(&self, key: &String) -> bool { '+getter_contract+' Self::prefix(self).get(key) == None } }')
    mutations['R18 local Resolution matches else collapse'] = ('lib.rs', lambda s: re.sub(r'if match package::canonical_entry\(pkg, target\) \{.*?\n            \} \{', 'if { let outcome = package::canonical_entry(pkg, target); if matches!(outcome, Resolution::One(_)) { false } else { true } } {', s, count=1, flags=re.S))
    for ty in ['Resolution<String>', 'HiddenResolution']:
        mutations['R18 Resolution parameter collapse '+ty] = ('lib.rs', lambda s,t=ty: s+'\ntype HiddenResolution = Resolution<String>; fn unregistered_resolution_absence(value: '+t+') -> bool { match value { Resolution::One(_) => false, Resolution::Absent | Resolution::Unknown(_) => true, Resolution::Ambiguous(_) => false } }')
    mutations['R18 Resolution Self collapse'] = ('resolution.rs', lambda s: s+'\nimpl<T> Resolution<T> { fn unregistered_resolution_absence(&self) -> bool { match self { Self::One(_) => false, Self::Absent | Self::Unknown(_) => true, Self::Ambiguous(_) => false } } }')
    mutations['R18 aliased bool cannot launder raw absence'] = ('package.rs', lambda s: s+'\ntype HiddenVerdict = (bool); fn unregistered_return_alias(entries: &[Entry], name: &str) -> HiddenVerdict { observation_contract! { rules: "-"; collections: "-"; scope: "raw absence control"; loss: "entry prefix"; fallback: "skip unknown"; package_loss: "independent"; } !entries.iter().any(|entry| entry.name == name) }')
    mutations['R18 opaque function cannot collapse package Resolution'] = ('lib.rs', lambda s: re.sub(r'if match package::canonical_entry\(pkg, target\) \{.*?\n            \} \{', 'if opaque_verdict(package::canonical_entry(pkg, target)) {', s, count=1, flags=re.S)+'\nfn opaque_verdict<T>(_value: T) -> bool { true }')
    # The new-accessor mutation must introduce a different access kind to test exact declaration matching.
    mutations['new undeclared accessor']=('lib.rs', lambda s:s.replace('acc.ran("CUSTOMXML-SCHEMA");','acc.ran("CUSTOMXML-SCHEMA"); let _ = new_collection.retain_budget(n, cap);',1))
    return mutations


def positive_cases(sources):
    controls={}
    def add(label, changed, presence=False):
        controls[label] = (changed, presence)
    # Positive corpus: every current production construct is parsed and audited above.
    # Additional non-consuming bodies exercise recognition without declarations.
    for index, control in enumerate([
        'fn multiple() { consume(|a| { a }, |b| { b }); consume(|| { 1 }); }',
        'fn nested() { consume(|a| consume(|b| { a + b })); }',
        'fn r#raw_name(r#value: u8) -> u8 { r#value }',
        'fn fresh\u0301(value\u0301: u8) -> u8 { value\u0301 } fn script(℘: u8) -> u8 { ℘ }',
        'macro_rules! make_item { ($name:ident) => { fn $name(x: u8) -> u8 { x } }; } make_item!(fresh);',
        'const C: usize = { 1 }; enum Extra { A = 1 } type Array = [u8; 1];',
        'fn boolean() { if a || !matches!(b, A | B) { return; } consume(|x| { x }); }',
        'const TEXT: &str = r#"|Package { entries, .. }|"#; // |Package { entries, .. }|',
        '/* outer /* inner */ |Package { entries, .. }| */ fn explicit() {}',
        '#[cfg(test)] mod tests { fn read() { consume(|Package { entries, .. }| entries.len()); } }',
        'const F: fn(u8) -> u8 = |x| x; static S: fn(u8) -> u8 = |x| (|y| y)(x);',
    ]):
        changed = dict(sources); changed['lib.rs'] += '\n' + control
        add('body/'+str(index), changed)
    # CompleteSet's production provider must keep the exact representation footprint
    # under all callable spellings, including a local function binding.
    for index, expression in enumerate(['Observed::is_complete(&self)', 'Self::is_complete(&self)', 'HiddenObservation::is_complete(&self)', '<Self>::is_complete(&self)', '<Observed<HashSet<K>>>::is_complete(&self)', '{ let test = Self::is_complete; test(&self) }']):
        changed = dict(sources)
        changed['observed.rs'] += '\ntype HiddenObservation<T> = Observed<T>;\n'
        changed['observed.rs'] = changed['observed.rs'].replace('if self.is_complete() {', 'if '+expression+' {', 1)
        add('provider/'+str(index), changed)
    changed = dict(sources)
    changed['lib.rs'] = changed['lib.rs'].replace('self.parts.values().flat_map(|p| p.special_notes.prefix().iter().cloned()).collect()', 'self.parts.values().flat_map(|p| { let retained = p.special_notes.prefix(); retained.iter().filter(|candidate| retained.contains(candidate)).cloned() }).collect()', 1)
    add('retained-prefix presence', changed)
    for attribute in ['inline', 'inline(always)', 'inline(never)', 'cold']:
        changed = dict(sources)
        changed['lib.rs'] += '\n#['+attribute+'] fn configured_entry_limit(limits: &Limits) -> usize { limits.entries }'
        add('attribute/'+attribute, changed)
    for index, body in enumerate(['std::format!("rule: {rule}")', '{ let v = std::vec![1, 2]; v.len().to_string() }', '{ std::debug_assert!(true); String::new() }', '{ let _ = std::matches!(true, true); String::new() }']):
        changed = dict(sources)
        changed['lib.rs'] += '\nfn display_rule(rule: &str) -> String { '+body+' }'
        add('standard/'+str(index), changed)
    for name in ['lib.rs', 'observed.rs', 'ownership.rs', 'resolution.rs', 'scan.rs']:
        changed = dict(sources)
        def qualified_derives(match):
            paths = ['std::fmt::Debug' if p.strip() == 'Debug' else 'serde::Serialize' if p.strip() == 'Serialize' else p.strip() for p in match[1].split(',')]
            return '#[derive('+', '.join(paths)+')]'
        changed[name] = re.sub(r'#\[derive\(([^)]*)\)\]', qualified_derives, changed[name])
        add('reader/'+name, changed)
    changed = dict(sources)
    getter_contract = 'observation_contract! { rules: "-"; collections: "access.prefix*1"; scope: "prefix presence control"; loss: "incomplete observed map"; fallback: "only presence is established"; package_loss: "independent"; }'
    changed['lib.rs'] += '\nimpl Observed<HashMap<String, String>> { fn declared_presence(&self, key: &String) -> bool { '+getter_contract+' Self::prefix(self).get(key).is_some() } }'
    add('declared-presence row', changed, presence=True)
    return controls



def verify_output(name, actual, expected):
    if actual != expected: raise ValueError(f'{name} drift; regenerate observation contracts')


# Frozen round-18 labels: deletion, duplication or an incomplete shard must fail.
MUTATION_INVENTORY = (
    'R18 Resolution Self collapse',
    'R18 Resolution parameter collapse HiddenResolution',
    'R18 Resolution parameter collapse Resolution<String>',
    'R18 Unknown-or-Absent',
    'R18 accessor function binding HiddenDefinitions::prefix',
    'R18 accessor function binding Self::prefix',
    'R18 aliased UFCS',
    'R18 aliased bool cannot launder raw absence',
    'R18 canonical external call',
    'R18 contracted raw iterator absence',
    'R18 contracted retained get equality absence',
    'R18 derive outside type context',
    'R18 direct state deletion',
    'R18 direct state negation',
    'R18 discovery complete deletion',
    'R18 discovery complete negation',
    'R18 fabricated absence',
    'R18 fabricated completeness',
    'R18 helper alias/value canonical_entry/hidden_lookup',
    'R18 helper alias/value canonical_entry/package::canonical_entry',
    'R18 helper alias/value content_unavailable/hidden_lookup',
    'R18 helper alias/value content_unavailable/package::content_unavailable',
    'R18 helper alias/value entry_by_name/hidden_lookup',
    'R18 helper alias/value entry_by_name/package::entry_by_name',
    'R18 helper-mediated absence',
    'R18 inline outside closed options',
    'R18 local Resolution matches else collapse',
    'R18 local prefix with exact original contract',
    'R18 missing Unknown',
    'R18 obsolete Option absence',
    'R18 opaque function cannot collapse package Resolution',
    'R18 qualified consuming macro arguments',
    'R18 raw iterator absence',
    'R18 raw slice alias',
    'R18 reader allowance cannot transfer into a function scope',
    'R18 reader assembly limit inversion',
    'R18 registry fields added to allowed DTO',
    'R18 renamed reader derive',
    'R18 retained map get absence',
    'R18 retained prefix !<Observed<HashSet<String>>>::prefix(self).contains(key)',
    'R18 retained prefix !<Observed<HashSet<String>>>::prefix(self).iter().any(|candidate| candidate == key)',
    'R18 retained prefix !<Self>::prefix(self).contains(key)',
    'R18 retained prefix !<Self>::prefix(self).iter().any(|candidate| candidate == key)',
    'R18 retained prefix !Self::prefix(self).contains(key)',
    'R18 retained prefix !Self::prefix(self).iter().any(|candidate| candidate == key)',
    'R18 retained prefix <Observed<HashSet<String>>>::prefix(self).contains(key) == false',
    'R18 retained prefix <Self>::prefix(self).contains(key) == false',
    'R18 retained prefix Self::prefix(self).contains(key) == false',
    'R18 retained prefix match <Observed<HashSet<String>>>::prefix(self).contains(key) { false => true, true => false }',
    'R18 retained prefix match <Self>::prefix(self).contains(key) { false => true, true => false }',
    'R18 retained prefix match Self::prefix(self).contains(key) { false => true, true => false }',
    'R18 retained prefix matches!(<Observed<HashSet<String>>>::prefix(self).contains(key), false)',
    'R18 retained prefix matches!(<Self>::prefix(self).contains(key), false)',
    'R18 retained prefix matches!(Self::prefix(self).contains(key), false)',
    'R18 same-named DTO in another module',
    'R18 serde field attribute outside policy',
    'R18 standard namespace import shadow',
    'R18 unauthorized ContentTypes reader Debug',
    'R18 unauthorized ContentTypes reader serde::Deserialize',
    'R18 unauthorized ContentTypes reader serde::Serialize',
    'R18 unauthorized reader on DTO',
    'R18 unsupported qualified env macro',
    'Unicode combining collection field',
    'Unicode combining membership receiver',
    'Unicode combining parser identifier',
    'accessor bypass',
    'adjacent inner destructured closure',
    'array-element destructured closure',
    'async destructured closure',
    'attribute macro on definition',
    'attribute macro on item invocation',
    'closure argument with nested pattern',
    'closure pattern after associated-type constraint',
    'closure pattern with attributes and comments',
    'closure with Unicode identifier continuation',
    'closure with generic struct path',
    'closure with nested block comment',
    'closure with parenthesized or-pattern',
    'closure with qualified struct path',
    'const fn-pointer destructured closure',
    'contract macro starts emitting code',
    'contract text in literal is not a declaration',
    'delta16 destructuring assignment',
    'delta16 macro-produced closure pattern',
    'delta16 module callback body',
    'delta16 parenthesized destructuring assignment',
    'delta16 raw field read',
    'delta16 raw registry method',
    'destructured closure returning adjacent closure',
    'destructured for read',
    'destructured match read',
    'destructured package read',
    'destructured parameter read',
    'enum discriminant body',
    'exception leakage',
    'external module without supplied source',
    'glob import could hide standard macro shadow',
    'immediately invoked destructured closure',
    'impl callback initializer',
    'let-bound destructured closure',
    'macro-captured collection field',
    'macro-generated consumer',
    'missing fallback',
    'module registry alias and method pointer',
    'module registry method pointer',
    'module registry type alias',
    'nested closure body',
    'nested struct closure pattern',
    'new undeclared accessor',
    'raw absence difference',
    'raw accessor',
    'raw membership bypass',
    'raw private representation',
    'raw registry function name',
    'raw registry type alias',
    'registry in representation provider',
    'representation membership bypass',
    'returned closure with later parameter',
    'seam/other-macro-path (R18 argument footprint)',
    'standard macro alias shadow',
    'standard macro import shadow',
    'standard macro shadow hides consuming expansion',
    'static callback body',
    'static fn-pointer destructured closure',
    'type-level constant body',
    'undeclared array consumer',
    'undeclared consumer',
    'undeclared representation consumer',
    'unsupported async body',
    'unsupported attribute macro',
    'unsupported expression macro',
    'unsupported foreign item',
    'unsupported macro repetition',
    'unsupported pattern macro in assignment',
    'unsupported pattern macro in binding',
    'unsupported pattern macro in match',
    'unsupported pattern macro in parameter',
    'unsupported production module',
    'unsupported trait body',
    'unsupported union item',
)
POSITIVE_INVENTORY = (
    'body/0',
    'body/1',
    'body/2',
    'body/3',
    'body/4',
    'body/5',
    'body/6',
    'body/7',
    'body/8',
    'body/9',
    'body/10',
    'provider/0',
    'provider/1',
    'provider/2',
    'provider/3',
    'provider/4',
    'provider/5',
    'retained-prefix presence',
    'attribute/inline',
    'attribute/inline(always)',
    'attribute/inline(never)',
    'attribute/cold',
    'standard/0',
    'standard/1',
    'standard/2',
    'standard/3',
    'reader/lib.rs',
    'reader/observed.rs',
    'reader/ownership.rs',
    'reader/resolution.rs',
    'reader/scan.rs',
    'declared-presence row',
)

def inventory_check(actual, expected, kind):
    counts = Counter(iter(actual))
    missing = sorted(set(expected)-counts.keys())
    extra = sorted(counts.keys()-set(expected))
    repeated = sorted(label for label, count in counts.items() if count != 1)
    if missing or extra or repeated:
        raise ValueError(f'{kind} inventory differs: missing={missing}; extra={extra}; repeated={repeated}')


def inventory_digest():
    return hashlib.sha256(('\n'.join(sorted(MUTATION_INVENTORY))+'\n').encode()).hexdigest()


def checked_mutations():
    mutations = mutation_cases()
    inventory_check(mutations, MUTATION_INVENTORY, 'mutation')
    if len(MUTATION_INVENTORY) != 140 or inventory_digest() != 'cb823adedb87fc9ba64516406abdf954635f7579ed2e695ae42599fcbd7f51d7':
        raise ValueError('pinned round-18 mutation inventory changed')
    return mutations


def reject_mutation(sources, label, case):
    name, mutation = case
    changed=dict(sources); changed[name]=mutation(changed[name])
    if changed[name] == sources[name]:
        raise ValueError('mutation did not change source: '+label)
    try:
        declarations(changed)
    except ValueError as error:
        message = str(error).splitlines()[0] if str(error) else ''
        if not message.strip():
            raise ValueError('mutation has no rejection message: '+label)
        return dict(label=label, message=message)
    raise ValueError('audit accepted mutation: '+label)


def accept_control(label, case, rows):
    changed, presence = case
    actual = declarations(changed)
    if presence:
        assert len(actual) == len(rows)+1, label
        assert next(r for r in actual if r['helper'] == 'lib.rs::declared_presence')['collections'] == 'access.prefix*1', label
    else:
        assert actual == rows, label


def check_controls(controls, rows):
    for label, case in controls.items():
        accept_control(label, case, rows)


def checked_controls(sources):
    controls=positive_cases(sources)
    inventory_check(controls, POSITIVE_INVENTORY, 'positive control')
    if len(POSITIVE_INVENTORY) != 32:
        raise ValueError('pinned round-18 positive-control count changed')
    return controls


def shard_cases(mutations, controls):
    return [('mutation', label) for label in mutations] + [('positive', label) for label in controls]


def artifact_self_test():
    # No assumption about hyphens, hashes, profiles, targets or a deps directory.
    for name in ('observation_recognizer-1111111111111111', 'observation_recognizer',
                 'observation-recognizer'):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            manifest=root/'Cargo.toml'
            binary=root/'custom-target/profile'/(name+('.exe' if os.name=='nt' else ''))
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b'fresh build')
            private=root/'private'; private.mkdir()
            stale=binary.parent/'stale'; stale.write_bytes(b'old build')
            item=dict(reason='compiler-artifact', manifest_path=str(manifest),
                      target=dict(name='observation-recognizer', kind=['bin']),
                      executable=str(binary))
            output=json.dumps(item)
            selected=recognizer_binary(recognizer_artifact(output, manifest), private)
            assert selected.parent==private
            binary.unlink()
            assert selected.read_bytes()==b'fresh build', 'private image depends on Cargo alias'
            for bad in ('', json.dumps(dict(item, executable=None)),
                        json.dumps(dict(item, manifest_path=str(root/'other/Cargo.toml'))),
                        output+'\n'+json.dumps(dict(item, executable=str(stale)))):
                try:
                    recognizer_artifact(bad, manifest)
                except ValueError:
                    pass
                else:
                    raise AssertionError('missing or ambiguous Cargo artifact was accepted')


def case_pool(rows, mutations, controls, jobs, timings):
    # Fresh interpreters own their recognizer pipes and content-addressed caches.
    # Files prevent blocked children; stdin carries only the parent's verified rows.
    cases=shard_cases(mutations, controls)
    with ExitStack() as stack:
        children=[]
        try:
            for index in range(jobs):
                stdin=stack.enter_context(tempfile.TemporaryFile(mode='w+t'))
                json.dump(rows, stdin); stdin.seek(0)
                stdout=stack.enter_context(tempfile.TemporaryFile(mode='w+t'))
                stderr=stack.enter_context(tempfile.TemporaryFile(mode='w+t'))
                command=[sys.executable, str(Path(__file__).resolve()), '--self-test',
                         '--shard', f'{index}/{jobs}', '--no-verify']
                if timings: command.append('--timings')
                process=subprocess.Popen(command, stdin=stdin, stdout=stdout, stderr=stderr)
                children.append((index, process, stdout, stderr))
            results=[]
            for index, process, stdout, stderr in children:
                code=process.wait()
                stdout.seek(0); stderr.seek(0)
                output=stdout.read(); errors=stderr.read()
                if code:
                    raise ValueError(f'shard {index}/{jobs} failed (exit {code}):\n{errors}\n{output}')
                shard=[]
                for line in output.splitlines():
                    item=json.loads(line)
                    if (not isinstance(item, dict) or set(item) != {'kind','label','message'}
                            or not isinstance(item['kind'], str) or item['kind'] not in {'mutation','positive'}
                            or not isinstance(item['label'], str) or not isinstance(item['message'], str)
                            or not item['message'].strip() or len(item['message'].splitlines()) != 1
                            or (item['kind'] == 'positive' and item['message'] != 'accepted')):
                        raise ValueError(f'shard {index}/{jobs} emitted an invalid rejection/control: {line}')
                    shard.append(item)
                inventory_check(((r['kind'],r['label']) for r in shard), cases[index::jobs], f'shard {index}/{jobs}')
                if timings:
                    count=sum(r['kind']=='mutation' for r in shard)
                    print(f'shard {index}/{jobs}: {count} mutations; {len(shard)-count} positive controls\n{errors}', file=sys.stderr)
                results.extend(shard)
            rejected=[r for r in results if r['kind']=='mutation']
            accepted=[r for r in results if r['kind']=='positive']
            inventory_check((r['label'] for r in rejected), MUTATION_INVENTORY, 'merged mutation')
            inventory_check((r['label'] for r in accepted), POSITIVE_INVENTORY, 'merged positive control')
            return {r['label']: r for r in rejected}
        finally:
            # A failed control or child cannot leave orphaned workers/recognizers running.
            for _, process, _, _ in children:
                if process.poll() is None: process.terminate()
            for _, process, _, _ in children:
                process.wait()


def self_test(sources, rows, jobs=1, timings=False):
    artifact_self_test()
    mutations=checked_mutations()
    controls=checked_controls(sources)
    if jobs == 1:
        results={label: reject_mutation(sources, label, case) for label, case in mutations.items()}
        inventory_check(results, MUTATION_INVENTORY, 'serial mutation')
        check_controls(controls, rows)
    else:
        results=case_pool(rows, mutations, controls, jobs, timings)
    # Preserve the pre-sharding stdout order, regardless of worker completion order.
    for label in mutations:
        print('rejected:', label, '-', results[label]['message'])
    print(f'{len(mutations)} source mutations rejected')
    print('accepted: current production corpus and non-consuming body controls')
    original=outputs(rows)
    try: verify_output('in-memory register', original[0]+'drift', original[0])
    except ValueError: print('rejected: generated register drift (exact byte comparison)')
    else: raise ValueError('audit accepted generated drift')

    assert declarations(sources)==rows, 'mutation audit contaminated the original sources'
    if timings:
        print(f'inventory: {len(mutations)} mutations; sha256={inventory_digest()}; {len(controls)} positive controls', file=sys.stderr)


def shard_value(value):
    try:
        index, count = map(int, value.split('/'))
    except ValueError:
        raise argparse.ArgumentTypeError('shard must be i/n (zero-based)')
    if not 1 <= count <= 4 or not 0 <= index < count:
        raise argparse.ArgumentTypeError('shard must satisfy 0 <= i < n <= 4')
    return index, count


def jobs_value(value):
    try:
        jobs=int(value)
    except ValueError:
        raise argparse.ArgumentTypeError('jobs must be an integer from 1 to 4')
    if not 1 <= jobs <= 4:
        raise argparse.ArgumentTypeError('jobs must be from 1 to 4')
    return jobs

def main():
    global _TIMINGS
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check',action='store_true')
    parser.add_argument('--self-test',action='store_true')
    parser.add_argument('--jobs',type=jobs_value,default=min(4, os.cpu_count() or 1),
                        help='mutation workers, 1 for sequential debugging (default: min(4, cores))')
    parser.add_argument('--shard',type=shard_value,help='internal worker/debug slice i/n (zero-based); verified parent rows on stdin')
    parser.add_argument('--no-verify',action='store_true',help=argparse.SUPPRESS)
    parser.add_argument('--timings',action='store_true',help='print timing and inventory details to stderr')
    args=parser.parse_args()
    _TIMINGS=args.timings
    if args.no_verify != (args.shard is not None) or (args.shard is not None and (not args.self_test or args.check)):
        parser.error('--no-verify is only for --self-test --shard i/n workers without --check')
    root=Path(__file__).resolve().parents[1]
    sources={p.name:p.read_text() for p in (root/'src').glob('*.rs')}
    start=time.perf_counter()
    if args.shard is not None:
        index, count=args.shard
        mutations=checked_mutations()
        controls=checked_controls(sources)
        # The verified parent sends its pristine rows over stdin. Workers never
        # re-audit the pristine tree or share a live recognizer pipe.
        rows=json.load(sys.stdin)
        if not isinstance(rows, list): raise ValueError('expected parent row list')
        for kind, label in shard_cases(mutations, controls)[index::count]:
            if kind == 'mutation':
                item=reject_mutation(sources, label, mutations[label])
                item['kind']='mutation'
            else:
                accept_control(label, controls[label], rows)
                item=dict(kind='positive', label=label, message='accepted')
            print(json.dumps(item))
        if args.timings: print(f'shard {index}/{count} wall: {time.perf_counter()-start:.3f}s', file=sys.stderr)
        return
    print(f'observation gate: cores={os.cpu_count() or 1} jobs={args.jobs}', file=sys.stderr)
    rows=declarations(sources)
    values=outputs(rows)
    files=[root/'observation-contracts.md',root/'src/contracts_generated.rs']
    # Verify currency before spending time on mutations, even on the default test path.
    for path,value in zip(files,values):
        if args.check or args.self_test:
            verify_output(path.name, path.read_text() if path.exists() else '', value)
        else: path.write_text(value)
    if args.self_test: self_test(sources, rows, args.jobs, args.timings)
    print(f'{len(rows)} declared bounded consumers; register and access audit passed')
    if args.timings:
        print(f'gate wall: {time.perf_counter()-start:.3f}s; units: {unit_audit.cache_info()}', file=sys.stderr)


if __name__=='__main__':
    try: main()
    except (ValueError,AssertionError,OSError,subprocess.SubprocessError) as error:
        print(error,file=sys.stderr);sys.exit(1)
