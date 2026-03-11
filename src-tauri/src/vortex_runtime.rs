//! QuickJS-based Vortex extension executor.
//!
//! Executes a Vortex game extension's `index.js` in an embedded QuickJS
//! sandbox with a mock `context` object. Intercepts `registerGame()`,
//! `registerModType()`, and `registerInstaller()` calls to extract
//! game support data as native Rust structs.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rquickjs::{context::EvalOptions, Array, Context, Function, Object, Runtime, Value};

use serde_json;

use crate::vortex_types::*;

/// Execute a Vortex extension and extract its registration data.
pub fn execute_extension(source: &ExtensionSource) -> Result<CapturedRegistrations, String> {
    let rt = Runtime::new().map_err(|e| format!("Failed to create QuickJS runtime: {e}"))?;
    rt.set_memory_limit(64 * 1024 * 1024);
    rt.set_max_stack_size(512 * 1024);

    // Install an interrupt handler that fires after a timeout.
    // This prevents malicious or buggy extensions from blocking forever.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    rt.set_interrupt_handler(Some(Box::new(move || std::time::Instant::now() > deadline)));

    let ctx = Context::full(&rt).map_err(|e| format!("Failed to create JS context: {e}"))?;

    ctx.with(|ctx| {
        let captured = Rc::new(RefCell::new(CapturedRegistrations::default()));

        setup_globals(&ctx, &captured, &source.extra_files)
            .map_err(|e| format!("Failed to setup globals: {e}"))?;

        let wrapped = format!(
            r#"
            (function() {{
                var module = {{ exports: {{}} }};
                var exports = module.exports;

                {source}

                var main = module.exports.default || module.exports;
                if (typeof main === 'function') {{
                    main(__vortex_context);
                }}
            }})();
            "#,
            source = source.index_js
        );

        let mut opts = EvalOptions::default();
        opts.strict = false;
        opts.global = true;

        match ctx.eval_with_options::<Value, _>(wrapped.as_bytes(), opts) {
            Ok(_) => {}
            Err(e) => {
                // Try to get the JS exception stack trace for better diagnostics
                let exception_detail = ctx
                    .catch()
                    .as_exception()
                    .and_then(|exc| {
                        let msg = exc.message().unwrap_or_default();
                        let stack = exc.stack().unwrap_or_default();
                        if stack.is_empty() {
                            Some(msg)
                        } else {
                            Some(format!("{}\n{}", msg, stack))
                        }
                    })
                    .unwrap_or_default();
                let err_msg = if exception_detail.is_empty() {
                    format!("JS execution error: {e}")
                } else {
                    format!("JS execution error: {}\n{}", e, exception_detail)
                };
                log::warn!("Extension execution warning: {}", err_msg);
            }
        }

        let result = captured.borrow().clone();
        Ok(result)
    })
}

// ---------------------------------------------------------------------------
// Global setup
// ---------------------------------------------------------------------------

fn setup_globals<'js>(
    ctx: &rquickjs::Ctx<'js>,
    captured: &Rc<RefCell<CapturedRegistrations>>,
    extra_files: &HashMap<String, String>,
) -> Result<(), String> {
    setup_console(ctx)?;
    setup_path_module(ctx)?;
    setup_require(ctx, extra_files)?;
    setup_process(ctx)?;
    setup_vortex_context(ctx, captured)?;
    Ok(())
}

fn setup_console(ctx: &rquickjs::Ctx<'_>) -> Result<(), String> {
    let console = Object::new(ctx.clone()).map_err(|e| e.to_string())?;
    console
        .set(
            "log",
            Function::new(ctx.clone(), |msg: String| {
                log::trace!("[vortex-ext] {}", msg);
            })
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    console
        .set(
            "warn",
            Function::new(ctx.clone(), |msg: String| {
                log::trace!("[vortex-ext WARN] {}", msg);
            })
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    console
        .set(
            "error",
            Function::new(ctx.clone(), |msg: String| {
                log::trace!("[vortex-ext ERROR] {}", msg);
            })
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    ctx.globals()
        .set("console", console)
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn setup_path_module(ctx: &rquickjs::Ctx<'_>) -> Result<(), String> {
    ctx.eval::<(), _>(
        r#"
        var __path_module = {
            join: function() {
                var parts = [];
                for (var i = 0; i < arguments.length; i++) {
                    if (arguments[i]) parts.push(arguments[i]);
                }
                return parts.join('/').replace(/\/+/g, '/');
            },
            dirname: function(p) {
                var idx = p.lastIndexOf('/');
                if (idx === -1) idx = p.lastIndexOf('\\');
                return idx >= 0 ? p.substring(0, idx) : '.';
            },
            basename: function(p, ext) {
                var base = p.replace(/.*[\/\\]/, '');
                if (ext && base.endsWith(ext)) base = base.slice(0, -ext.length);
                return base;
            },
            extname: function(p) {
                var idx = p.lastIndexOf('.');
                return idx >= 0 ? p.substring(idx) : '';
            },
            sep: '\\',
            resolve: function() {
                return __path_module.join.apply(null, arguments);
            },
            isAbsolute: function(p) {
                return /^[A-Z]:/i.test(p) || p.startsWith('/');
            },
            relative: function(from, to) { return to; },
            normalize: function(p) { return p; },
            parse: function(p) {
                return { root: '', dir: __path_module.dirname(p), base: __path_module.basename(p),
                         ext: __path_module.extname(p), name: __path_module.basename(p, __path_module.extname(p)) };
            },
            format: function(obj) { return (obj.dir || '') + '/' + (obj.base || ''); },
            posix: null,
            win32: null,
        };
        __path_module.posix = __path_module;
        __path_module.win32 = __path_module;
    "#,
    )
    .map_err(|e| format!("Failed to setup path module: {e}"))?;
    Ok(())
}

fn setup_require(
    ctx: &rquickjs::Ctx<'_>,
    extra_files: &HashMap<String, String>,
) -> Result<(), String> {
    let mut module_js = String::from(
        r#"
        var __require_cache = {};

        // Base component class that can be extended by webpack bundles
        class __MockComponent {
            constructor(props) { this.props = props || {}; this.state = {}; this.context = {}; }
            setState(s) { Object.assign(this.state, typeof s === 'function' ? s(this.state) : s); }
            forceUpdate() {}
            render() { return null; }
        }
        // Stub layout component
        function __MockFlexLayout() {}
        __MockFlexLayout.Fixed = function() {};
        __MockFlexLayout.Flex = function() {};
        __MockFlexLayout.type = 'row';

        var __mock_modules = {
            'path': __path_module,
            'vortex-api': {
                fs: {
                    // Async methods
                    statAsync: function() { return Promise.resolve({ isDirectory: function() { return false; }, isFile: function() { return true; }, isSymbolicLink: function() { return false; }, mtime: new Date() }); },
                    readdirAsync: function() { return Promise.resolve([]); },
                    readFileAsync: function() { return Promise.resolve(''); },
                    writeFileAsync: function() { return Promise.resolve(); },
                    ensureDirAsync: function() { return Promise.resolve(); },
                    ensureDirWritableAsync: function() { return Promise.resolve(); },
                    removeAsync: function() { return Promise.resolve(); },
                    copyAsync: function() { return Promise.resolve(); },
                    renameAsync: function() { return Promise.resolve(); },
                    linkAsync: function() { return Promise.resolve(); },
                    symlinkAsync: function() { return Promise.resolve(); },
                    unlinkAsync: function() { return Promise.resolve(); },
                    lstatAsync: function() { return Promise.resolve({ isDirectory: function() { return false; }, isFile: function() { return true; }, isSymbolicLink: function() { return false; } }); },
                    // Sync methods (used by webpack bundles)
                    readdirSync: function() { return []; },
                    readFileSync: function() { return ''; },
                    existsSync: function() { return false; },
                    statSync: function() { return { isDirectory: function() { return false; }, isFile: function() { return true; }, mtime: new Date() }; },
                    mkdirSync: function() {},
                    writeFileSync: function() {},
                },
                util: {
                    steam: { findByAppId: function() { return Promise.resolve(''); } },
                    GameStoreHelper: {
                        findByAppId: function() { return Promise.resolve({ gamePath: '' }); },
                        isGameInstalled: function() { return Promise.resolve(false); },
                    },
                    getVortexPath: function() { return ''; },
                    log: function() {},
                    opn: function() {},
                    toPromise: function(fn) { return fn; },
                    toBlue: function(fn) { return fn; },
                    makeReactive: function(obj) { return obj; },
                    getSafe: function(state, path, def) { return def; },
                    setSafe: function(state, path, val) { return state; },
                    merge: function(obj, key, val) { return obj; },
                    renderModName: function(mod) { return (mod && mod.id) || ''; },
                    installPathForGame: function() { return ''; },
                    getNormalizeFunc: function() { return Promise.resolve(function(s) { return s ? s.toLowerCase() : ''; }); },
                    getGame: function() { return { getInstalledVersion: function() { return Promise.resolve('1.0.0'); }, name: '', executable: function() { return ''; } }; },
                    copyFileAtomic: function() { return Promise.resolve(); },
                    delayed: function(ms) { return new Promise(function(r) { r(); }); },
                    walk: function() { return Promise.resolve(); },
                    getManifest: function() { return Promise.resolve({ files: [] }); },
                    ConcurrencyLimiter: function ConcurrencyLimiter() { this.do = function(fn) { return fn(); }; },
                    Debouncer: function Debouncer() { this.schedule = function() {}; this.clear = function() {}; this.runNow = function() { return Promise.resolve(); }; },
                    BatchDispatch: function BatchDispatch() { this.finish = function() {}; },
                    LazyComponent: function LazyComponent() { return function() {}; },
                    UserCanceled: (function() { function UC() { this.name = 'UserCanceled'; } UC.prototype = Object.create(Error.prototype); return UC; })(),
                    NotSupportedError: (function() { function NS() { this.name = 'NotSupportedError'; } NS.prototype = Object.create(Error.prototype); return NS; })(),
                    NotFound: (function() { function NF() { this.name = 'NotFound'; } NF.prototype = Object.create(Error.prototype); return NF; })(),
                    ProcessCanceled: (function() { function PC() { this.name = 'ProcessCanceled'; } PC.prototype = Object.create(Error.prototype); return PC; })(),
                    DataInvalid: (function() { function DI() { this.name = 'DataInvalid'; } DI.prototype = Object.create(Error.prototype); return DI; })(),
                    SetupError: (function() { function SE() { this.name = 'SetupError'; } SE.prototype = Object.create(Error.prototype); return SE; })(),
                },
                selectors: {
                    activeGameId: function() { return ''; },
                    gamePath: function() { return ''; },
                    discoveryByGame: function() { return {}; },
                    installPathForGame: function() { return ''; },
                    modPathsForGame: function() { return {}; },
                    profileById: function() { return {}; },
                    activeProfile: function() { return {}; },
                    currentGame: function() { return {}; },
                    gameById: function() { return {}; },
                    currentGameDiscovery: function() { return {}; },
                    knownGames: function() { return []; },
                    modById: function() { return undefined; },
                    installPath: function() { return ''; },
                    downloadPath: function() { return ''; },
                    currentActivator: function() { return undefined; },
                },
                types: {},
                actions: {
                    setModType: function() {},
                    setModAttribute: function() {},
                    setModEnabled: function() {},
                    addMod: function() {},
                    removeMod: function() {},
                    setDownloadModInfo: function() {},
                    setDeploymentNecessary: function() {},
                    setNextProfile: function() {},
                    setModsEnabled: function() {},
                    setFeature: function() {},
                    dismissNotification: function() {},
                    setLoadOrder: function() {},
                },
                log: function() {},
                // UI component stubs — must be valid constructors for class inheritance
                ComponentEx: __MockComponent,
                PureComponentEx: __MockComponent,
                MainPage: __MockComponent,
                FlexLayout: __MockFlexLayout,
                Icon: function Icon() {},
                FormInput: function FormInput() {},
                ToolbarIcon: function ToolbarIcon() {},
                Toggle: function Toggle() {},
                Spinner: function Spinner() {},
                More: function More() {},
                Steps: function Steps() {},
                Table: function Table() {},
                tooltip: { Button: function() {}, IconButton: function() {} },
                EmptyPlaceholder: function EmptyPlaceholder() {},
                Usage: function Usage() {},
            },
            'electron': {
                app: {
                    getVersion: function() { return '1.10.0'; },
                    getPath: function(name) {
                        if (name === 'documents') return 'C:\\Users\\user\\Documents';
                        if (name === 'appData') return 'C:\\Users\\user\\AppData\\Roaming';
                        return 'C:\\Users\\user';
                    },
                },
                remote: {
                    app: {
                        getVersion: function() { return '1.10.0'; },
                        getPath: function(name) {
                            if (name === 'documents') return 'C:\\Users\\user\\Documents';
                            if (name === 'appData') return 'C:\\Users\\user\\AppData\\Roaming';
                            return 'C:\\Users\\user';
                        },
                    },
                },
            },
            'bluebird': (function() {
                var P = typeof Promise !== 'undefined' ? Promise : function() {};
                P.resolve = P.resolve || function(v) { return new Promise(function(res) { res(v); }); };
                P.reject = P.reject || function(e) { return new Promise(function(_, rej) { rej(e); }); };
                P.method = P.method || function(fn) { return fn; };
                P.try = P.try || function(fn) { try { return P.resolve(fn()); } catch(e) { return P.reject(e); } };
                P.coroutine = P.coroutine || function(fn) { return fn; };
                P.each = P.each || function(arr, fn) { return P.resolve(arr.forEach(fn)); };
                P.reduce = P.reduce || function(arr, fn, init) { return P.resolve(arr.reduce(fn, init)); };
                P.map = P.map || function(arr, fn) { return P.resolve((arr || []).map(fn)); };
                P.filter = P.filter || function(arr, fn) { return P.resolve((arr || []).filter(fn)); };
                P.props = P.props || function(obj) { return P.resolve(obj); };
                P.delay = P.delay || function(ms) { return new P(function(r) { r(); }); };
                return P;
            })(),
            'react': {
                createElement: function() { return {}; },
                Component: function() {},
                PureComponent: function() {},
                Fragment: 'Fragment',
            },
            'react-dom': {},
            'semver': {
                satisfies: function() { return true; },
                valid: function(v) { return v; },
                coerce: function(v) { return { version: typeof v === 'string' ? v : '1.0.0', major: 1, minor: 0, patch: 0 }; },
                gte: function() { return true; },
                gt: function() { return true; },
                lt: function() { return false; },
                lte: function() { return true; },
                eq: function() { return true; },
                compare: function() { return 0; },
            },
            'winreg': function() { this.get = function(_, cb) { cb && cb('not available'); }; },
            'turbowalk': function() { return Promise.resolve([]); },
            'exe-version': {
                getFileVersion: function() { return Promise.resolve('1.0.0'); },
                getFileVersionLocalized: function() { return Promise.resolve('1.0.0'); },
            },
            'lodash': (function() {
                var _ = function(v) { return v; };
                _.get = function(obj, path, def) { return def; };
                _.set = function() {};
                _.merge = function() { return arguments[0] || {}; };
                _.cloneDeep = function(v) { return JSON.parse(JSON.stringify(v || {})); };
                _.uniq = function(arr) { return arr; };
                _.includes = function(arr, v) { return arr && arr.indexOf(v) >= 0; };
                _.noop = function() {};
                _.debounce = function(fn) { return fn; };
                _.isEqual = function(a, b) { return a === b; };
                return _;
            })(),
            'url': { parse: function(u) { return { href: u }; }, format: function(u) { return u; } },
            'util': { promisify: function(fn) { return fn; }, inspect: function(v) { return String(v); } },
            'https': { get: function() {} },
            'crypto': { createHash: function() { return { update: function() { return this; }, digest: function() { return ''; } }; } },
            'xml2js': { parseString: function(s, cb) { cb && cb(null, {}); }, Builder: function() { this.buildObject = function() { return ''; }; }, Parser: function() { this.parseString = function(s, cb) { cb && cb(null, {}); }; this.parseStringPromise = function() { return Promise.resolve({}); }; } },
            'shortid': { generate: function() { return 'id'; } },
            'fs': { readFileSync: function() { return ''; }, writeFileSync: function() {}, existsSync: function() { return false; }, statSync: function() { return { isDirectory: function() { return false; } }; } },
            'child_process': { execSync: function() { return ''; }, spawn: function() { return { on: function() {}, stdout: { on: function() {} }, stderr: { on: function() {} } }; } },
            'redux-act': { createAction: function() { return function() {}; }, createReducer: function() { return function() {}; } },
            'react-bootstrap': {},
            'react-i18next': { withTranslation: function() { return function(c) { return c; }; } },
            'react-redux': { connect: function() { return function(c) { return c; }; } },
            'relaxed-json': { parse: function(s) { try { return JSON.parse(s); } catch(e) { return {}; } } },
            'winapi-bindings': { GetModuleHandle: function() { return 0; } },
            'vortex-parse-ini': { IniParser: function() { this.read = function() { return Promise.resolve({}); }; this.write = function() { return Promise.resolve(); }; } },
        };
    "#,
    );

    // Define require() FIRST so extra files can use it during their evaluation
    module_js.push_str(
        r#"
        var __extra_file_sources = {};

        function require(name) {
            if (__require_cache[name]) return __require_cache[name];
            if (__mock_modules[name]) {
                __require_cache[name] = __mock_modules[name];
                return __mock_modules[name];
            }
            var base = name.replace(/\.js$/, '');
            if (__mock_modules[base]) {
                __require_cache[name] = __mock_modules[base];
                return __mock_modules[base];
            }
            if (__mock_modules['./' + name]) {
                __require_cache[name] = __mock_modules['./' + name];
                return __mock_modules['./' + name];
            }
            // Lazy-load extra files: evaluate source on first require
            var srcKey = name;
            if (!__extra_file_sources[srcKey]) {
                srcKey = base;
            }
            if (!__extra_file_sources[srcKey]) {
                srcKey = './' + name;
            }
            if (!__extra_file_sources[srcKey]) {
                srcKey = './' + base;
            }
            if (__extra_file_sources[srcKey]) {
                var mod = { exports: {} };
                var src = __extra_file_sources[srcKey];
                delete __extra_file_sources[srcKey]; // prevent infinite recursion
                try {
                    var fn = new Function('module', 'exports', 'require', '__filename', '__dirname', src);
                    fn(mod, mod.exports, require, srcKey + '.js', '.');
                } catch(e) {
                    console.warn('Error evaluating extra file "' + srcKey + '": ' + e);
                }
                __require_cache[name] = mod.exports;
                __mock_modules[name] = mod.exports;
                __mock_modules[base] = mod.exports;
                __mock_modules['./' + base] = mod.exports;
                return mod.exports;
            }
            console.warn('require: unknown module "' + name + '", returning empty object');
            var empty = {};
            __require_cache[name] = empty;
            return empty;
        }
    "#,
    );

    // Register extra file sources for lazy evaluation
    for (name, content) in extra_files {
        let module_name = name.trim_end_matches(".js");
        // Use JSON encoding to safely embed the source string
        let json_encoded = serde_json::to_string(content).unwrap_or_else(|_| "\"\"".to_string());
        module_js.push_str(&format!(
            "__extra_file_sources['./{module_name}'] = {json_encoded};\n",
            module_name = module_name,
            json_encoded = json_encoded,
        ));
    }

    ctx.eval::<(), _>(module_js.as_str())
        .map_err(|e| format!("Failed to setup require: {e}"))?;
    Ok(())
}

fn setup_process(ctx: &rquickjs::Ctx<'_>) -> Result<(), String> {
    ctx.eval::<(), _>(
        r#"
        var process = {
            platform: 'win32',
            env: { ProgramFiles: 'C:\\Program Files', 'ProgramFiles(x86)': 'C:\\Program Files (x86)' },
            cwd: function() { return 'C:\\'; },
        };
        var __filename = 'index.js';
        var __dirname = '.';
        if (typeof Buffer === 'undefined') {
            var Buffer = { from: function(s) { return s; }, alloc: function(n) { return ''; }, isBuffer: function() { return false; } };
        }
    "#,
    )
    .map_err(|e| format!("Failed to setup process: {e}"))?;
    Ok(())
}

fn setup_vortex_context<'js>(
    ctx: &rquickjs::Ctx<'js>,
    captured: &Rc<RefCell<CapturedRegistrations>>,
) -> Result<(), String> {
    let context_obj = Object::new(ctx.clone()).map_err(|e| e.to_string())?;

    // -- registerGame --
    let cap = captured.clone();
    context_obj
        .set(
            "registerGame",
            Function::new(
                ctx.clone(),
                move |spec: Value<'_>| -> rquickjs::Result<()> {
                    let ctx = spec.ctx();
                    match extract_game_registration(ctx, &spec) {
                        Ok(reg) => {
                            log::info!(
                                "Vortex extension registered game: {} ({})",
                                reg.name,
                                reg.id
                            );
                            cap.borrow_mut().game = Some(reg);
                        }
                        Err(e) => {
                            log::warn!("Failed to extract game registration: {}", e);
                        }
                    }
                    Ok(())
                },
            )
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;

    // -- registerGameStub --
    let cap = captured.clone();
    context_obj
        .set(
            "registerGameStub",
            Function::new(
                ctx.clone(),
                move |spec: Value<'_>, _ext_info: Value<'_>| -> rquickjs::Result<()> {
                    let ctx = spec.ctx();
                    match extract_game_registration(ctx, &spec) {
                        Ok(mut reg) => {
                            reg.is_stub = true;
                            log::info!(
                                "Vortex extension registered game stub: {} ({})",
                                reg.name,
                                reg.id
                            );
                            cap.borrow_mut().game = Some(reg);
                        }
                        Err(e) => {
                            log::warn!("Failed to extract game stub: {}", e);
                        }
                    }
                    Ok(())
                },
            )
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;

    // -- registerModType --
    // Uses JS-side wrapper to call getPath() and extract the result as a string,
    // then passes that string to the Rust callback. This avoids lifetime issues
    // with holding both Ctx and Value params in the same closure.
    let cap = captured.clone();
    let register_mod_type_inner = Function::new(
        ctx.clone(),
        move |id: String, priority: i32, target_path: String| -> rquickjs::Result<()> {
            let relative = target_path
                .replace("C:\\game\\", "")
                .replace("C:\\game", ".")
                .replace("C:/game/", "")
                .replace("C:/game", ".");

            // Sanitize: reject paths that escape the game directory
            let sanitized =
                if relative.contains("..") || relative.starts_with('/') || relative.contains(":\\")
                {
                    log::warn!(
                        "Mod type '{}' returned unsafe path '{}', falling back to '.'",
                        id,
                        relative
                    );
                    ".".to_string()
                } else {
                    relative
                };

            log::info!("Registered mod type: {} -> {}", id, sanitized);
            cap.borrow_mut().mod_types.push(VortexModType {
                id,
                priority,
                target_path: sanitized,
            });
            Ok(())
        },
    )
    .map_err(|e| e.to_string())?;
    // Set __registerModTypeInner as a global so the JS wrapper can find it
    ctx.globals()
        .set("__registerModTypeInner", register_mod_type_inner)
        .map_err(|e| e.to_string())?;
    ctx.eval::<(), _>(
        r#"
        var __vortex_context_registerModType = function(id, priority, isSupported, getPath, test) {
            var targetPath = '.';
            if (typeof getPath === 'function') {
                try {
                    var mockGame = { path: 'C:\\game', modPath: '.', name: 'Game', mergeMods: true };
                    var result = getPath(mockGame);
                    if (typeof result === 'string') targetPath = result;
                } catch(e) {}
            }
            __registerModTypeInner(id, priority, targetPath);
        };
    "#,
    )
    .map_err(|e| format!("Failed to setup registerModType: {e}"))?;
    let register_mod_type_fn: Value = ctx
        .eval("__vortex_context_registerModType")
        .map_err(|e| e.to_string())?;
    context_obj
        .set("registerModType", register_mod_type_fn)
        .map_err(|e| e.to_string())?;

    // -- registerInstaller --
    let cap = captured.clone();
    context_obj
        .set(
            "registerInstaller",
            Function::new(
                ctx.clone(),
                move |id: String, priority: i32| -> rquickjs::Result<()> {
                    log::info!("Registered installer: {} (priority {})", id, priority);
                    cap.borrow_mut()
                        .installers
                        .push(VortexInstallerMeta { id, priority });
                    Ok(())
                },
            )
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;

    // -- No-op stubs for other registration methods --
    for name in &[
        "registerAction",
        "registerLoadOrder",
        "registerReducer",
        "registerSettings",
        "registerMigration",
        "registerTableAttribute",
        "registerProfileFeature",
        "registerDeploymentMethod",
        "registerMerge",
        "registerTest",
        "registerMainPage",
        "registerDashlet",
        "registerOverlay",
        "registerDialog",
        "registerBanner",
        "registerAttributeExtractor",
        "registerProfileFile",
        "registerModSource",
        "registerLoadOrderPage",
    ] {
        context_obj
            .set(
                *name,
                Function::new(ctx.clone(), || -> rquickjs::Result<()> { Ok(()) })
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
    }

    // -- once / optional / requireExtension --
    for name in &["once", "optional", "requireExtension"] {
        context_obj
            .set(
                *name,
                Function::new(ctx.clone(), || -> rquickjs::Result<()> { Ok(()) })
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
    }

    // -- context.api --
    ctx.eval::<(), _>(
        r#"
        var __vortex_api_mock = {
            getState: function() {
                return {
                    settings: { gameMode: { discovered: {} }, mods: { paths: {} } },
                    persistent: { profiles: {}, mods: {} },
                    session: { base: { activity: {} } },
                };
            },
            store: {
                getState: function() { return __vortex_api_mock.getState(); },
                dispatch: function() {},
            },
            events: {
                on: function() { return __vortex_api_mock.events; },
                emit: function() {},
                removeListener: function() {},
            },
            translate: function(s) { return typeof s === 'string' ? s : (s && s.toString ? s.toString() : ''); },
            sendNotification: function() {},
            showErrorNotification: function() {},
            showDialog: function() { return Promise.resolve({ action: 'Cancel' }); },
            suppressNotification: function() {},
            getPath: function(name) {
                if (name === 'documents') return 'C:\\Users\\user\\Documents';
                if (name === 'appData') return 'C:\\Users\\user\\AppData\\Roaming';
                return 'C:\\Users\\user';
            },
            ext: { addMetaServer: function() {} },
            setStylesheet: function() {},
            locale: function() { return 'en'; },
            onStateChange: function() {},
            onAsync: function() {},
        };
    "#,
    )
    .map_err(|e| format!("Failed to setup API mock: {e}"))?;

    let api: Value = ctx.eval("__vortex_api_mock").map_err(|e| e.to_string())?;
    context_obj.set("api", api).map_err(|e| e.to_string())?;

    ctx.globals()
        .set("__vortex_context", context_obj)
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Extraction helpers
// ---------------------------------------------------------------------------

fn extract_game_registration<'js>(
    ctx: &rquickjs::Ctx<'js>,
    spec: &Value<'js>,
) -> Result<VortexGameRegistration, String> {
    let obj = Object::from_value(spec.clone())
        .map_err(|_| "registerGame argument is not an object".to_string())?;

    let id: String = obj
        .get("id")
        .map_err(|_| "Missing 'id' field in registerGame".to_string())?;

    let name: String = obj.get("name").unwrap_or_else(|_| id.clone());

    let executable = extract_string_or_fn(ctx, &obj, "executable").unwrap_or_default();

    let required_files: Vec<String> = obj
        .get::<_, Vec<String>>("requiredFiles")
        .unwrap_or_default();

    let query_mod_path =
        extract_string_or_fn(ctx, &obj, "queryModPath").unwrap_or_else(|| ".".to_string());

    let merge_mods = obj.get::<_, bool>("mergeMods").unwrap_or(true);

    let environment = obj
        .get::<_, Value>("environment")
        .ok()
        .and_then(|v| {
            let json_str = ctx.json_stringify(v).ok()??;
            let s = json_str.to_string().ok()?;
            serde_json::from_str(&s).ok()
        })
        .unwrap_or(serde_json::Value::Null);

    let details = obj
        .get::<_, Value>("details")
        .ok()
        .and_then(|v| {
            let json_str = ctx.json_stringify(v).ok()??;
            let s = json_str.to_string().ok()?;
            serde_json::from_str(&s).ok()
        })
        .unwrap_or(serde_json::Value::Null);

    let mut store_ids = extract_store_ids(&environment, &details);

    // Extract store IDs from queryArgs (modern Vortex pattern)
    // queryArgs: { steam: [{ id: '489830' }], gog: [{ id: '...' }], ... }
    if let Ok(qa) = obj.get::<_, Value>("queryArgs") {
        if let Ok(qa_obj) = Object::from_value(qa) {
            extract_query_args_store_ids(ctx, &qa_obj, &mut store_ids);
        }
    }

    let supported_tools = extract_tools(ctx, &obj);
    let steam_dir_name = extract_steam_dir_name(&details);

    Ok(VortexGameRegistration {
        id,
        name,
        executable,
        required_files,
        query_mod_path,
        merge_mods,
        store_ids,
        details,
        environment,
        supported_tools,
        mod_types: Vec::new(),
        installers: Vec::new(),
        is_stub: false,
        steam_dir_name,
    })
}

fn extract_string_or_fn<'js>(
    _ctx: &rquickjs::Ctx<'js>,
    obj: &Object<'js>,
    key: &str,
) -> Option<String> {
    let val: Value = obj.get(key).ok()?;

    if val.is_string() {
        return val.as_string().and_then(|s| s.to_string().ok());
    }

    if val.is_function() {
        let func = Function::from_value(val).ok()?;
        if let Ok(result) = func.call::<_, Value>(()) {
            if let Some(s) = result.as_string().and_then(|s| s.to_string().ok()) {
                return Some(s);
            }
        }
        if let Ok(result) = func.call::<_, Value>(("C:\\game",)) {
            if let Some(s) = result.as_string().and_then(|s| s.to_string().ok()) {
                return Some(s);
            }
        }
    }

    None
}

fn extract_store_ids(environment: &serde_json::Value, details: &serde_json::Value) -> StoreIds {
    let steam_from_env = environment
        .get("SteamAPPId")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let steam_from_details = details.get("steamAppId").and_then(|v| {
        v.as_str()
            .map(|s| s.to_string())
            .or_else(|| v.as_u64().map(|n| n.to_string()))
    });

    StoreIds {
        steam_app_id: steam_from_env.or(steam_from_details),
        gog_app_id: details
            .get("gogAppId")
            .and_then(|v| v.as_str().map(|s| s.to_string())),
        epic_app_id: details
            .get("epicAppId")
            .and_then(|v| v.as_str().map(|s| s.to_string())),
        xbox_id: details
            .get("xboxId")
            .and_then(|v| v.as_str().map(|s| s.to_string())),
    }
}

fn extract_tools<'js>(ctx: &rquickjs::Ctx<'js>, obj: &Object<'js>) -> Vec<VortexTool> {
    let tools_val: Value = match obj.get("supportedTools") {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    if !tools_val.is_array() {
        return Vec::new();
    }

    let arr = match Array::from_value(tools_val) {
        Ok(a) => a,
        Err(_) => return Vec::new(),
    };

    let mut tools = Vec::new();
    for i in 0..arr.len() {
        let tool_val: Value = match arr.get(i) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let tool_obj = match Object::from_value(tool_val) {
            Ok(o) => o,
            Err(_) => continue,
        };

        let id: String = match tool_obj.get("id") {
            Ok(v) => v,
            Err(_) => continue,
        };
        let name: String = tool_obj.get("name").unwrap_or_else(|_| id.clone());
        let executable = extract_string_or_fn(ctx, &tool_obj, "executable").unwrap_or_default();
        let required_files: Vec<String> = tool_obj.get("requiredFiles").unwrap_or_default();
        let short_name: Option<String> = tool_obj.get("shortName").ok();
        let relative: bool = tool_obj.get("relative").unwrap_or(false);
        let exclusive: bool = tool_obj.get("exclusive").unwrap_or(false);
        let default_primary: bool = tool_obj.get("defaultPrimary").unwrap_or(false);
        let parameters: Vec<String> = tool_obj.get("parameters").unwrap_or_default();

        tools.push(VortexTool {
            id,
            name,
            executable,
            required_files,
            short_name,
            relative,
            exclusive,
            default_primary,
            parameters,
        });
    }

    tools
}

/// Extract store IDs from queryArgs object.
/// Format: queryArgs: { steam: [{ id: '489830' }], gog: [{ id: '...' }], xbox: [...], epic: [...] }
fn extract_query_args_store_ids<'js>(
    _ctx: &rquickjs::Ctx<'js>,
    qa: &Object<'js>,
    store_ids: &mut StoreIds,
) {
    // Helper: extract first ID from a store array like [{ id: '489830' }]
    let get_first_id = |key: &str| -> Option<String> {
        let val: Value = qa.get(key).ok()?;
        if !val.is_array() {
            return None;
        }
        let arr = Array::from_value(val).ok()?;
        if arr.len() == 0 {
            return None;
        }
        let first: Value = arr.get(0).ok()?;
        let obj = Object::from_value(first).ok()?;
        obj.get::<_, String>("id").ok()
    };

    if store_ids.steam_app_id.is_none() {
        store_ids.steam_app_id = get_first_id("steam");
    }
    if store_ids.gog_app_id.is_none() {
        store_ids.gog_app_id = get_first_id("gog");
    }
    if store_ids.epic_app_id.is_none() {
        store_ids.epic_app_id = get_first_id("epic");
    }
    if store_ids.xbox_id.is_none() {
        store_ids.xbox_id = get_first_id("xbox");
    }
}

fn extract_steam_dir_name(details: &serde_json::Value) -> Option<String> {
    details
        .get("steamDirName")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
}
