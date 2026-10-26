use std::collections::{HashMap, HashSet};
use std::fs;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use boa_engine::ast::visitor::{VisitWith, Visitor, VisitorMut};
use boa_engine::ast::Script as AstScript;
use boa_engine::interner::{Interner, Sym};
use boa_engine::script::Script;
use boa_engine::{Context, JsResult, Source};

const CACHE_VERSION: u32 = 3;

struct SymExtractor<'a> {
    interner: &'a Interner,
    symbols: Vec<(Sym, String)>,
    seen: HashSet<Sym>,
}

impl<'a> SymExtractor<'a> {
    fn new(interner: &'a Interner) -> Self {
        Self {
            interner,
            symbols: Vec::new(),
            seen: HashSet::new(),
        }
    }
}

impl<'ast> Visitor<'ast> for SymExtractor<'_> {
    type BreakTy = ();

    fn visit_sym(&mut self, node: &'ast Sym) -> ControlFlow<()> {
        if self.seen.insert(*node) {
            if let Some(resolved) = self.interner.resolve(*node) {
                self.symbols.push((*node, resolved.to_string()));
            }
        }
        ControlFlow::Continue(())
    }
}

struct SymRemapper<'a> {
    mapping: &'a HashMap<Sym, Sym>,
}

impl<'ast> VisitorMut<'ast> for SymRemapper<'_> {
    type BreakTy = ();

    fn visit_sym_mut(&mut self, node: &'ast mut Sym) -> ControlFlow<()> {
        if let Some(&new_sym) = self.mapping.get(node) {
            *node = new_sym;
        }
        ControlFlow::Continue(())
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
pub enum CachedScriptPayload {
    Bytecode {
        version: u32,
        source_hash: u64,
        codeblock: boa_engine::vm::SerializedCodeBlock,
    },
    Ast {
        version: u32,
        source_hash: u64,
        ast: AstScript,
        symbols: Vec<(Sym, String)>,
    },
}

impl CachedScriptPayload {
    fn is_valid(&self, expected_hash: u64) -> bool {
        match self {
            Self::Bytecode { version, source_hash, .. }
            | Self::Ast { version, source_hash, .. } => {
                *version == CACHE_VERSION && *source_hash == expected_hash
            }
        }
    }
}

#[derive(Debug)]
pub struct CodeCache {
    disk_dir: Option<PathBuf>,
    memory_cache: Mutex<HashMap<u64, std::sync::Arc<[u8]>>>,
}

static DEFAULT_CODE_CACHE: std::sync::OnceLock<std::sync::Arc<CodeCache>> = std::sync::OnceLock::new();

pub fn default_code_cache() -> std::sync::Arc<CodeCache> {
    DEFAULT_CODE_CACHE
        .get_or_init(|| std::sync::Arc::new(CodeCache::default()))
        .clone()
}

impl CodeCache {
    
    pub fn new(disk_dir: Option<PathBuf>) -> Self {
        if let Some(ref dir) = disk_dir {
            let _ = fs::create_dir_all(dir);
        }
        Self {
            disk_dir,
            memory_cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn default_dir() -> PathBuf {
        if let Ok(dir) = std::env::var("RIX_CODE_CACHE_DIR") {
            if !dir.is_empty() {
                return PathBuf::from(dir);
            }
        }
        if let Ok(runtime_dir) = std::env::var("XDG_CACHE_HOME") {
            if !runtime_dir.is_empty() {
                return PathBuf::from(runtime_dir).join("rixbrowser").join("code_cache");
            }
        }
        if let Ok(home) = std::env::var("HOME") {
            if !home.is_empty() {
                return PathBuf::from(home)
                    .join(".cache")
                    .join("rixbrowser")
                    .join("code_cache");
            }
        }
        std::env::temp_dir().join("rix-code-cache")
    }

    pub fn hash_source(source: &str, _path: Option<&Path>) -> u64 {
        const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
        const FNV_PRIME: u64 = 0x100000001b3;

        let mut hash = FNV_OFFSET_BASIS;
        for &byte in source.as_bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(FNV_PRIME);
        }
        hash
    }

    fn instantiate_payload(
        &self,
        payload: CachedScriptPayload,
        source: &str,
        path: Option<&Path>,
        context: &mut Context,
    ) -> JsResult<Script> {
        match payload {
            CachedScriptPayload::Bytecode { codeblock, .. } => {
                Ok(Script::from_serialized_codeblock(
                    &codeblock,
                    Some(source),
                    None,
                    path.map(Path::to_path_buf),
                    context,
                ))
            }
            CachedScriptPayload::Ast { mut ast, symbols, .. } => {
                let mut mapping = HashMap::with_capacity(symbols.len());
                for (old_sym, s) in &symbols {
                    let new_sym = context.interner_mut().get_or_intern(s.as_str());
                    mapping.insert(*old_sym, new_sym);
                }
                let mut remapper = SymRemapper { mapping: &mapping };
                let _ = ast.visit_with_mut(&mut remapper);

                Script::from_ast(
                    ast,
                    Some(source),
                    None,
                    path.map(Path::to_path_buf),
                    context,
                )
            }
        }
    }

    pub fn load_or_parse(
        &self,
        source: &str,
        path: Option<&Path>,
        context: &mut Context,
    ) -> JsResult<(Script, bool)> {
        let hash = Self::hash_source(source, path);

        if let Ok(guard) = self.memory_cache.lock() {
            if let Some(bytes) = guard.get(&hash) {
                if let Ok(payload) = postcard::from_bytes::<CachedScriptPayload>(bytes) {
                    if payload.is_valid(hash) {
                        if let Ok(script) = self.instantiate_payload(payload, source, path, context) {
                            tracing::debug!(hash = format_args!("{:016x}", hash), "code cache L1 memory hit");
                            return Ok((script, true));
                        }
                    }
                }
            }
        }

        if let Some(ref dir) = self.disk_dir {
            let file_path = dir.join(format!("{:016x}.bin", hash));
            if let Ok(bytes) = fs::read(&file_path) {
                if let Ok(payload) = postcard::from_bytes::<CachedScriptPayload>(&bytes) {
                    if payload.is_valid(hash) {
                        if let Ok(script) = self.instantiate_payload(payload, source, path, context) {
                            tracing::debug!(hash = format_args!("{:016x}", hash), "code cache L2 disk hit");
                            if let Ok(mut guard) = self.memory_cache.lock() {
                                guard.insert(hash, bytes.into());
                            }
                            return Ok((script, true));
                        }
                    }
                }
            }
        }

        let source_bytes = Source::from_bytes(source.as_bytes());
        let script = match path {
            Some(path) => Script::parse(source_bytes.with_path(path), None, context)?,
            None => Script::parse(source_bytes, None, context)?,
        };

        let payload = match script.codeblock(context) {
            Ok(cb) => CachedScriptPayload::Bytecode {
                version: CACHE_VERSION,
                source_hash: hash,
                codeblock: cb.to_serialized(),
            },
            Err(_) => {
                if let Some(ast) = script.ast() {
                    let symbols = {
                        let mut extractor = SymExtractor::new(context.interner());
                        let _ = ast.visit_with(&mut extractor);
                        extractor.symbols
                    };
                    CachedScriptPayload::Ast {
                        version: CACHE_VERSION,
                        source_hash: hash,
                        ast,
                        symbols,
                    }
                } else {
                    return Ok((script, false));
                }
            }
        };

        if let Ok(serialized) = postcard::to_allocvec(&payload) {
            
            if let Some(ref dir) = self.disk_dir {
                let temp_path = dir.join(format!("{:016x}.tmp.{}", hash, std::process::id()));
                let final_path = dir.join(format!("{:016x}.bin", hash));
                if fs::write(&temp_path, &serialized).is_ok() {
                    let _ = fs::rename(&temp_path, &final_path);
                }
            }

            if let Ok(mut guard) = self.memory_cache.lock() {
                guard.insert(hash, serialized.into());
            }
        }

        Ok((script, false))
    }

    pub fn clear_memory(&self) {
        if let Ok(mut guard) = self.memory_cache.lock() {
            guard.clear();
        }
    }
}

impl Default for CodeCache {
    fn default() -> Self {
        Self::new(Some(Self::default_dir()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_code_cache_roundtrip() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache = CodeCache::new(Some(temp_dir.path().to_path_buf()));

        let source = r#"
            function factorial(n) {
                if (n <= 1) return 1;
                return n * factorial(n - 1);
            }
            var testObj = { greeting: "hello", value: factorial(5) };
            testObj.value;
        "#;

        let result1 = {
            let mut context = Context::default();
            let (script, from_cache) = cache.load_or_parse(source, None, &mut context).unwrap();
            assert!(!from_cache, "First load should be a cache miss");
            let res = script.evaluate(&mut context).unwrap();
            res.to_number(&mut context).unwrap()
        };
        assert_eq!(result1, 120.0);

        cache.clear_memory();

        let result2 = {
            let mut context = Context::default();
            
            context.interner_mut().get_or_intern("unrelated_foo");
            context.interner_mut().get_or_intern("unrelated_bar");

            let (script, from_cache) = cache.load_or_parse(source, None, &mut context).unwrap();
            assert!(from_cache, "Second load should hit the L2 disk cache");
            let res = script.evaluate(&mut context).unwrap();
            res.to_number(&mut context).unwrap()
        };
        assert_eq!(result2, 120.0);

        let result3 = {
            let mut context = Context::default();
            context.interner_mut().get_or_intern("another_symbol");

            let (script, from_cache) = cache.load_or_parse(source, None, &mut context).unwrap();
            assert!(from_cache, "Third load should hit the L1 memory cache");
            let res = script.evaluate(&mut context).unwrap();
            res.to_number(&mut context).unwrap()
        };
        assert_eq!(result3, 120.0);

        let yt_snippet = r#"
            var ytcfg={d:function(){return window.yt&&yt.config_||ytcfg.data_||(ytcfg.data_={})},get:function(k,o){return k in ytcfg.d()?ytcfg.d()[k]:o},set:function(){var a=arguments;if(a.length>1)ytcfg.d()[a[0]]=a[1];else{var k;for(k in a[0])ytcfg.d()[k]=a[0][k]}}};
            window.ytcfg.set('EMERGENCY_BASE_URL', '/error_204?t=jserror');
        "#;
        let mut context = Context::default();
        let (script, from_cache) = cache.load_or_parse(yt_snippet, None, &mut context).unwrap();
        assert!(!from_cache, "First load must be a miss");
        let _ = script.evaluate(&mut context);

        let (script2, from_cache2) = cache.load_or_parse(yt_snippet, None, &mut context).unwrap();
        assert!(from_cache2, "Second load MUST hit cache!");
        let cb_start = std::time::Instant::now();
        let _ = script2.codeblock(&mut context).unwrap();
        let cb_duration = cb_start.elapsed();
        assert_eq!(cb_duration.as_millis(), 0, "Cached script codeblock took {:?}, expected 0ms", cb_duration);
        let _ = script2.evaluate(&mut context);

        cache.clear_memory();
        let (script3, from_cache3) = cache.load_or_parse(yt_snippet, None, &mut context).unwrap();
        assert!(from_cache3, "Third load MUST hit L2 disk cache!");
        let cb_start3 = std::time::Instant::now();
        let _ = script3.codeblock(&mut context).unwrap();
        let cb_duration3 = cb_start3.elapsed();
        assert_eq!(cb_duration3.as_millis(), 0, "Disk cached script codeblock took {:?}, expected 0ms", cb_duration3);
        let _ = script3.evaluate(&mut context);

        let path1 = Path::new("https://www.youtube.com/s/desktop/app.js?v=1");
        let path2 = Path::new("https://youtube.com/s/desktop/app.js?v=2");
        assert_eq!(
            CodeCache::hash_source(yt_snippet, Some(path1)),
            CodeCache::hash_source(yt_snippet, Some(path2))
        );
        let (script_path_hit, from_cache_path) =
            cache.load_or_parse(yt_snippet, Some(path2), &mut context).unwrap();
        assert!(from_cache_path, "Load with different path should hit cache");
        let _ = script_path_hit.evaluate(&mut context);
    }
}
