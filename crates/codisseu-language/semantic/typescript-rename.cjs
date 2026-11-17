// Own, read-only integration with the explicitly configured TypeScript runtime.
// No workspace source/configuration is executed and no workspace file is read.
'use strict';
const fs = require('node:fs');
const path = require('node:path');
const started = Date.now();
let work = 0;
let fallbackPath = '';
const output = { coverage: 'Unsupported', conflicts: [], issues: [], missingReferences: [], note: 'Prova TypeScript limitada aos snapshots e ao runtime autorizado; falhas e referências não resolvidas bloqueiam.', compilerVersion: '', checkedDeclarations: 0 };
const issue = (message, file, range) => {
  if (output.issues.length < 256) output.issues.push({ path: file || fallbackPath, ...(range ? { range } : {}), message });
};
const conflict = (message, file, range) => {
  if (output.conflicts.length < 256) output.conflicts.push({ path: file || '', range, message });
};
function budget(amount = 1) {
  work += amount;
  if (work > 500000 || Date.now() - started > 8500) throw new Error('Limite de nós/tempo do verificador TypeScript excedido');
}
function absolute(value) {
  if (typeof value !== 'string' || !path.isAbsolute(value)) throw new Error('Caminho absoluto obrigatório');
  return path.normalize(value);
}
function offset(text, line, column) {
  if (!Number.isSafeInteger(line) || !Number.isSafeInteger(column) || line < 0 || column < 0) throw new Error('Posição UTF-16 inválida');
  let start = 0;
  for (let i = 0; i < line; i++) { budget(); const n = text.indexOf('\n', start); if (n < 0) throw new Error('Linha UTF-16 ausente'); start = n + 1; }
  let end = text.indexOf('\n', start); if (end < 0) end = text.length;
  if (end > start && text[end - 1] === '\r') end--;
  const value = start + column;
  if (value > end || (value > 0 && /[\uDC00-\uDFFF]/.test(text[value] || '') && /[\uD800-\uDBFF]/.test(text[value - 1]))) throw new Error('Coluna divide UTF-16 ou excede linha');
  return value;
}
function range(text, start, end) {
  function pos(value) { const prefix = text.slice(0, value); const at = prefix.lastIndexOf('\n'); return [(prefix.match(/\n/g) || []).length, value - at - 1]; }
  const [sl, sc] = pos(start), [el, ec] = pos(end);
  return { start_line: sl, start_character: sc, end_line: el, end_character: ec };
}
function main(input) {
  fallbackPath = absolute(input.origin.path);
  const moduleFile = absolute(input.typeScriptModule);
  if (path.basename(moduleFile) !== 'typescript.js' || path.basename(path.dirname(moduleFile)) !== 'lib') throw new Error('Runtime explícito deve apontar typescript/lib/typescript.js');
  const runtime = fs.realpathSync(moduleFile);
  const ts = require(runtime);
  output.compilerVersion = ts.version;
  const libDir = path.dirname(runtime);
  const root = absolute(input.root);
  if (!ts.isIdentifierText(input.newName, ts.ScriptTarget.Latest, ts.LanguageVariant.Standard) || input.newName.length > 256) throw new Error('Nome TypeScript inválido');
  const files = new Map();
  const snapshotPaths = new Set();
  if (!Array.isArray(input.snapshots) || input.snapshots.length === 0 || input.snapshots.length > 64) throw new Error('Corpus exige 1–64 snapshots');
  let bytes = 0, editCount = 0;
  for (const original of input.snapshots) {
    budget();
    const name = absolute(original.path);
    if (snapshotPaths.has(name)) throw new Error('Snapshot duplicado');
    snapshotPaths.add(name);
    if (name !== root && !name.startsWith(root + path.sep)) throw new Error('Snapshot fora da raiz autorizada');
    if (typeof original.before !== 'string' || typeof original.after !== 'string' || !Array.isArray(original.edits)) throw new Error('Snapshot inválido');
    const size = Buffer.byteLength(original.before) + Buffer.byteLength(original.after);
    bytes += size; editCount += original.edits.length;
    if (Buffer.byteLength(original.before) > 1048576 || Buffer.byteLength(original.after) > 1048576 || bytes > 8388608 || editCount > 1024) throw new Error('Limite de snapshots/edições TypeScript excedido');
    if (!/\.(?:ts|tsx|js|jsx)$/.test(name)) {
      if (original.edits.length || original.before !== original.after) throw new Error('Proposta mistura linguagens');
      continue;
    }
    if (files.has(name)) throw new Error('Snapshot duplicado');
    const edits = original.edits.map(edit => {
      budget(); const r = edit.range;
      const start = offset(original.before, r.start_line, r.start_character), end = offset(original.before, r.end_line, r.end_character);
      if (start >= end || typeof edit.newText !== 'string' && typeof edit.new_text !== 'string') throw new Error('Edição vazia/inválida');
      const text = edit.newText ?? edit.new_text;
      if (text.length > 1024) throw new Error('Substituição excede limite');
      return { start, end, text, range: r };
    }).sort((a, b) => a.start - b.start);
    let result = original.before, delta = 0;
    for (let i = 0; i < edits.length; i++) {
      const edit = edits[i];
      if (i && edits[i - 1].end > edit.start) throw new Error('Edições sobrepostas');
      edit.afterStart = edit.start + delta; edit.afterEnd = edit.afterStart + edit.text.length;
      delta += edit.text.length - (edit.end - edit.start);
    }
    for (const edit of [...edits].reverse()) result = result.slice(0, edit.start) + edit.text + result.slice(edit.end);
    if (result !== original.after) throw new Error('Snapshot after não corresponde às edições exatas');
    files.set(name, { ...original, path: name, edits });
  }
  if (files.size === 0) throw new Error('Nenhum snapshot TypeScript');
  const origin = absolute(input.origin.path), source = files.get(origin);
  if (!source) throw new Error('Origem ausente do corpus TypeScript');
  const originOffset = offset(source.before, input.origin.line, input.origin.character);
  const configName = input.configName ?? 'tsconfig.json';
  if (!['tsconfig.json', 'jsconfig.json'].includes(configName)) throw new Error('Nome de configuração TypeScript/JavaScript inválido');
  let options = ts.getDefaultCompilerOptions();
  if (input.configText !== null && input.configText !== undefined) {
    if (typeof input.configText !== 'string' || input.configText.length > 1048576) throw new Error('Configuração JSON excede limite');
    const parsed = ts.parseConfigFileTextToJson(path.join(root, configName), input.configText);
    if (parsed.error) throw new Error('Configuração TypeScript não é JSON/JSONC válido');
    const config = parsed.config;
    if (!config || typeof config !== 'object' || config.extends || config.references || config.compilerOptions?.plugins?.length || config.compilerOptions?.types?.length || config.compilerOptions?.typeRoots?.length) throw new Error('Extends/references/plugins/types externos não disponíveis no corpus');
    const converted = ts.convertCompilerOptionsFromJson(config.compilerOptions || {}, root, configName);
    if (converted.errors.length) throw new Error(ts.flattenDiagnosticMessageText(converted.errors[0].messageText, '\n'));
    options = { ...options, ...converted.options };
  } else if (configName === 'jsconfig.json') {
    throw new Error('jsconfig.json identificado sem snapshot de configuração');
  } else if ([...files.keys()].some(name => /\.(?:js|jsx)$/.test(name))) {
    // An inferred editor project admits JavaScript without enabling type-error
    // reporting. These are host options, not a generated workspace config.
    options = { ...options, allowJs: true, allowNonTsExtensions: true, allowSyntheticDefaultImports: true };
  }
  options = { ...options, noEmit: true, noEmitOnError: true, skipLibCheck: false, types: [], plugins: [], incremental: false, composite: false };
  if ([...files.keys()].some(name => /\.(?:js|jsx)$/.test(name))
    && !(options.allowJs ?? options.checkJs ?? false)) throw new Error('Consumidor JavaScript no corpus exige allowJs na configuração real; não pode ser omitido');
  const libCache = new Map(), unresolved = new Set();
  function standardFile(name) {
    name = path.normalize(name);
    return path.dirname(name) === libDir && /^lib(?:\.[a-z0-9.-]+)?\.d\.ts$/i.test(path.basename(name));
  }
  function library(name) {
    if (!standardFile(name)) return undefined;
    if (libCache.has(name)) return libCache.get(name);
    // Only standard libraries shipped beside the authorized compiler may be read.
    let text;
    try { const actual = fs.realpathSync(name); if (path.dirname(actual) !== libDir || !standardFile(actual)) return undefined; text = fs.readFileSync(actual, 'utf8'); } catch { return undefined; }
    libCache.set(name, text); return text;
  }
  function host(after) {
    const readFile = name => { name = path.normalize(name); const f = files.get(name); return f ? (after ? f.after : f.before) : library(name); };
    const host = {
      getCompilationSettings: () => options, getCurrentDirectory: () => root,
      getScriptFileNames: () => [...files.keys()], getScriptVersion: () => after ? '2' : '1',
      getScriptSnapshot: name => { const text = readFile(name); return text === undefined ? undefined : ts.ScriptSnapshot.fromString(text); },
      getDefaultLibFileName: opts => path.join(libDir, path.basename(ts.getDefaultLibFileName(opts))),
      readFile, fileExists: name => readFile(name) !== undefined,
      directoryExists: name => { name = path.normalize(name); return name === libDir || [...files.keys()].some(f => f.startsWith(name + path.sep)); },
      getDirectories: () => [], readDirectory: () => [], realpath: name => path.normalize(name),
      useCaseSensitiveFileNames: () => true, getNewLine: () => '\n',
      resolveModuleNames: (names, containing) => names.map(name => {
        budget(); const resolved = ts.resolveModuleName(name, containing, options, host).resolvedModule;
        if (!resolved || !files.has(path.normalize(resolved.resolvedFileName))) { unresolved.add(`${containing}: ${name}`); return undefined; }
        return resolved;
      }),
    };
    return host;
  }
  function service(after) {
    const service = ts.createLanguageService(host(after));
    const program = service.getProgram(); if (!program) throw new Error('Programa TypeScript indisponível');
    for (const sf of program.getSourceFiles()) {
      budget();
      if (!files.has(path.normalize(sf.fileName)) && !standardFile(sf.fileName)) throw new Error('Compilador tentou carregar arquivo externo ao corpus');
      if (files.has(path.normalize(sf.fileName))) {
        if (sf.checkJsDirective?.enabled === false) throw new Error('Diretiva @ts-nocheck impede prova de diagnósticos no corpus');
        if (sf.commentDirectives?.length) throw new Error('Diretiva @ts-ignore/@ts-expect-error suprime diagnósticos necessários à prova');
        if (sf.typeReferenceDirectives.length) throw new Error('Referência de tipos externa não resolvida');
        for (const reference of sf.referencedFiles) {
          const target = path.resolve(path.dirname(sf.fileName), reference.fileName);
          if (!files.has(target)) throw new Error('Triple-slash reference fora do corpus');
        }
      }
    }
    return { service, program, checker: program.getTypeChecker() };
  }
  const before = service(false);
  if (unresolved.size) throw new Error(`Imports fora do corpus: ${[...unresolved].slice(0, 8).join('; ')}`);
  function diagnostics(state, after) {
    const diags = [...state.program.getOptionsDiagnostics(), ...state.program.getGlobalDiagnostics(), ...state.program.getSyntacticDiagnostics(), ...state.program.getSemanticDiagnostics()];
    for (const d of diags) {
      budget(); if (d.category !== ts.DiagnosticCategory.Error) continue;
      const f = d.file && files.get(path.normalize(d.file.fileName));
      const message = `TS${d.code}: ${ts.flattenDiagnosticMessageText(d.messageText, '\n')}`;
      if (!after) issue(`Diagnóstico anterior impede prova: ${message}`, f?.path, f && range(f.before, d.start || 0, (d.start || 0) + (d.length || 0)));
      else if (f) {
        const start = inverse(f, d.start || 0) ?? f.edits[0]?.start ?? 0;
        const end = inverse(f, (d.start || 0) + (d.length || 0)) ?? start;
        conflict(`Proposta introduz diagnóstico: ${message}`, f.path, range(f.before, start, Math.max(start, end)));
      } else issue(`Diagnóstico do compilador: ${message}`);
    }
  }
  diagnostics(before, false);
  if (output.issues.length) return;
  const info = before.service.getRenameInfo(origin, originOffset, { allowRenameOfImportPath: false });
  if (!info.canRename) throw new Error(info.localizedErrorMessage || 'Compilador não reconheceu símbolo renomeável');
  const locations = before.service.findRenameLocations(origin, originOffset, false, false, true);
  if (!locations?.length || locations.length > 1024) throw new Error('Conjunto de referências ausente ou excede limite');
  const declarationKinds = new Set([
    ts.SyntaxKind.ClassDeclaration, ts.SyntaxKind.ClassExpression, ts.SyntaxKind.InterfaceDeclaration,
    ts.SyntaxKind.TypeAliasDeclaration, ts.SyntaxKind.TypeParameter, ts.SyntaxKind.FunctionDeclaration,
    ts.SyntaxKind.FunctionExpression, ts.SyntaxKind.MethodDeclaration, ts.SyntaxKind.MethodSignature,
    ts.SyntaxKind.PropertyDeclaration, ts.SyntaxKind.PropertySignature, ts.SyntaxKind.Parameter,
    ts.SyntaxKind.VariableDeclaration, ts.SyntaxKind.BindingElement, ts.SyntaxKind.ImportSpecifier,
    ts.SyntaxKind.ImportClause, ts.SyntaxKind.NamespaceImport, ts.SyntaxKind.ImportEqualsDeclaration,
    ts.SyntaxKind.ExportSpecifier, ts.SyntaxKind.EnumDeclaration, ts.SyntaxKind.EnumMember,
    ts.SyntaxKind.GetAccessor, ts.SyntaxKind.SetAccessor, ts.SyntaxKind.ModuleDeclaration,
  ]);
  const declarations = new Set();
  const expected = new Map();
  for (const location of locations) {
    budget(); const name = path.normalize(location.fileName), f = files.get(name);
    if (!f) throw new Error('Rename requer referência fora do corpus');
    const start = location.textSpan.start, end = start + location.textSpan.length;
    const text = (location.prefixText || '') + input.newName + (location.suffixText || '');
    const key = `${name}:${start}:${end}`;
    const sf = before.program.getSourceFile(name);
    const token = ts.getTokenAtPosition(sf, start);
    if (token.getStart(sf) === start && token.end === end && token.parent.name === token
      && declarationKinds.has(token.parent.kind)) declarations.add(key);
    if (expected.has(key) && expected.get(key).text !== text) throw new Error('Referência do compilador ambígua');
    expected.set(key, { text, prefix: location.prefixText || '', suffix: location.suffixText || '' });
  }
  // Include the exact compiler declaration set even when a call is omitted:
  // this metadata must remain stable while the provider fills missing uses.
  output.checkedDeclarations = declarations.size;
  for (const [name, f] of files) {
    for (const edit of f.edits) {
      budget(); const key = `${name}:${edit.start}:${edit.end}`, wanted = expected.get(key);
      if (!wanted || wanted.text !== edit.text) conflict('Edição não coincide com a referência e substituição exatas do compilador', name, edit.range);
      else { edit.prefix = wanted.prefix; edit.suffix = wanted.suffix; expected.delete(key); }
    }
  }
  for (const key of expected.keys()) {
    const [endText, startText, ...segments] = key.split(':').reverse(); const name = segments.reverse().join(':');
    const f = files.get(name);
    const missing = { path: name, range: range(f.before, Number(startText), Number(endText)) };
    // These are origins for another real provider query, never replacement
    // edits. The original proposal remains rejected until all bindings pass.
    if (output.missingReferences.length < 256) output.missingReferences.push(missing);
    conflict('Proposta omite referência comprovada pelo compilador', name, missing.range);
  }
  if (output.conflicts.length) { output.coverage = 'Rejected'; return; }
  function inverse(file, position) {
    let delta = 0;
    for (const edit of file.edits) {
      if (position < edit.afterStart) break;
      if (position === edit.afterStart) return edit.start;
      if (position === edit.afterEnd) return edit.end;
      if (position < edit.afterEnd) return null;
      delta += edit.text.length - (edit.end - edit.start);
    }
    return position - delta;
  }
  function forward(file, position) {
    let delta = 0;
    for (const edit of file.edits) {
      if (position < edit.start) break;
      if (position === edit.start) return edit.afterStart;
      if (position === edit.end) return edit.afterEnd;
      if (position < edit.end) return null;
      delta += edit.text.length - (edit.end - edit.start);
    }
    return position + delta;
  }
  function symbolIdentity(symbol, state, after, seen = new Set()) {
    budget();
    if (!symbol || seen.has(symbol)) return null;
    seen.add(symbol);
    const declarations = symbol.declarations;
    if (!declarations?.length) return null;
    const ids = [];
    for (const declaration of declarations) {
      budget(); const sf = declaration.getSourceFile(), name = path.normalize(sf.fileName), f = files.get(name);
      if (!f && !standardFile(name)) throw new Error('Símbolo depende de declaração externa ao corpus');
      const start = after && f ? inverse(f, declaration.getStart(sf)) : declaration.getStart(sf);
      if (start === null) return null;
      const kind = ts.isPropertyAssignment(declaration) || ts.isShorthandPropertyAssignment(declaration) ? 'object-property' : declaration.kind;
      ids.push(`${name}:${start}:${kind}`);
    }
    if (symbol.flags & ts.SymbolFlags.Alias) {
      const target = symbolIdentity(state.checker.getAliasedSymbol(symbol), state, after, seen);
      if (!target) return null; ids.push(`alias:${target}`);
    }
    return ids.sort().join('|');
  }
  function tokens(state, after) {
    const result = new Map();
    for (const [name, f] of files) {
      const sf = state.program.getSourceFile(name); if (!sf) throw new Error('Snapshot ausente do programa');
      const list = new Map();
      const visit = node => {
        budget();
        if (ts.isPropertyAccessExpression(node) || ts.isElementAccessExpression(node)) {
          const type = state.checker.getTypeAtLocation(node.expression);
          if (type.flags & (ts.TypeFlags.Any | ts.TypeFlags.Unknown)) issue('Receptor any/unknown impede provar referências dinâmicas', name, range(f.before, after ? (inverse(f, node.getStart(sf)) ?? 0) : node.getStart(sf), after ? (inverse(f, node.getStart(sf)) ?? 0) : node.getStart(sf)));
          if (ts.isElementAccessExpression(node) && (!node.argumentExpression || !ts.isStringLiteralLike(node.argumentExpression))) issue('Chave computada não literal exige resolução adicional', name);
        }
        if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && ['eval', 'Function'].includes(node.expression.text)) issue('Avaliação dinâmica impede prova de referências', name);
        if (ts.isIdentifier(node) || ts.isPrivateIdentifier(node) || ts.isStringLiteralLike(node) && (ts.isElementAccessExpression(node.parent) || ts.isPropertySignature(node.parent) || ts.isPropertyDeclaration(node.parent))) {
          const start = node.getStart(sf), end = node.end;
          const symbol = state.checker.getSymbolAtLocation(node);
          const identity = symbolIdentity(symbol, state, after);
          const valueSymbol = ts.isShorthandPropertyAssignment(node.parent) && node.parent.name === node
            ? symbolIdentity(state.checker.getShorthandAssignmentValueSymbol(node.parent), state, after) : null;
          list.set(`${start}:${end}`, { node, symbol: identity, valueSymbol, start, end });
        }
        ts.forEachChild(node, visit);
      };
      visit(sf); result.set(name, list);
    }
    return result;
  }
  const originalTokens = tokens(before, false);
  if (output.issues.length) return;
  // Compiler rename locations intentionally omit runtime string keys. Certifying
  // ordinary bindings alone would silently break Reflect.get(a, 'old'), JSON
  // serialization, key enumeration or spreads of the affected object. Relate
  // those operations to the exact member declarations, including imported and
  // inherited types; equal spelling on an unrelated object is insufficient.
  const affectedMembers = new Set();
  for (const [name, list] of originalTokens) {
    for (const token of list.values()) {
      const edit = files.get(name).edits.find(e => e.start === token.start && e.end === token.end);
      const symbol = before.checker.getSymbolAtLocation(token.node);
      if (edit && !edit.prefix && symbol && symbol.flags & (ts.SymbolFlags.Property | ts.SymbolFlags.Method | ts.SymbolFlags.GetAccessor | ts.SymbolFlags.SetAccessor)) affectedMembers.add(token.symbol);
    }
  }
  function affectedType(type, seen = new Set(), depth = 0) {
    budget();
    if (!type || seen.has(type)) return false;
    if (depth > 16) throw new Error('Tipo de reflexão excede profundidade comprovada');
    seen.add(type);
    if (type.isUnionOrIntersection?.() && type.types.some(t => affectedType(t, seen, depth + 1))) return true;
    const constraint = before.checker.getBaseConstraintOfType(type);
    if (constraint && constraint !== type && affectedType(constraint, seen, depth + 1)) return true;
    if (type.typeArguments?.some(t => affectedType(t, seen, depth + 1))) return true;
    for (const property of before.checker.getPropertiesOfType(type)) {
      budget();
      if (affectedMembers.has(symbolIdentity(property, before, false))) return true;
      // Traverse nested user objects, not the complete standard library graph.
      const declaration = property.valueDeclaration || property.declarations?.[0];
      if (declaration && files.has(path.normalize(declaration.getSourceFile().fileName))
        && affectedType(before.checker.getTypeOfSymbolAtLocation(property, declaration), seen, depth + 1)) return true;
    }
    return false;
  }
  function standardCallable(node) {
    let declaration = before.checker.getResolvedSignature(node)?.declaration;
    // A contextual function type can hide the standard signature of an alias.
    // Follow concrete initializers, with a cycle/depth bound. Even if reassigned,
    // a possible reflective target warrants rejecting this proof.
    let expression = node.expression;
    if (ts.isPropertyAccessExpression(expression) && ['call', 'apply', 'bind'].includes(expression.name.text)) {
      expression = expression.expression;
      declaration = before.checker.getTypeAtLocation(expression).getCallSignatures()[0]?.declaration;
    }
    const seen = new Set();
    for (let depth = 0; depth < 16 && (!declaration || !standardFile(declaration.getSourceFile().fileName)); depth++) {
      budget();
      const symbol = before.checker.getSymbolAtLocation(expression);
      const value = symbol?.valueDeclaration;
      if (!value || seen.has(value) || !ts.isVariableDeclaration(value) || !value.initializer) break;
      seen.add(value); expression = value.initializer;
      declaration = before.checker.getTypeAtLocation(expression).getCallSignatures()[0]?.declaration;
    }
    if (!declaration || !standardFile(declaration.getSourceFile().fileName)) return null;
    let parent = declaration.parent;
    while (parent && !ts.isModuleDeclaration(parent) && !ts.isInterfaceDeclaration(parent)) parent = parent.parent;
    return { owner: parent?.name?.text, name: declaration.name?.text };
  }
  if (affectedMembers.size) for (const [name, f] of files) {
    const sf = before.program.getSourceFile(name);
    const relevant = expression => expression && affectedType(before.checker.getTypeAtLocation(expression));
    const reflectiveArgument = expression => {
      const type = before.checker.getTypeAtLocation(expression);
      // Erasure to object/{}/a generic parameter cannot exclude the affected
      // instance. This is narrower than treating unrelated concrete B as A.
      return relevant(expression) || !!(type.flags & (ts.TypeFlags.Any | ts.TypeFlags.Unknown | ts.TypeFlags.NonPrimitive | ts.TypeFlags.TypeParameter))
        || !!(type.objectFlags & ts.ObjectFlags.Anonymous) && before.checker.getPropertiesOfType(type).length === 0;
    };
    const visit = node => {
      budget();
      let unsafe = false;
      if (ts.isCallExpression(node)) {
        if (!before.checker.getResolvedSignature(node)?.declaration
          && node.arguments.some(reflectiveArgument)) issue('Chamada não resolvida recebe objeto afetado e exige prova adicional de comportamento em runtime', name, range(f.before, node.getStart(sf), node.end));
        const callable = standardCallable(node);
        if (callable) {
          const reflection = callable.owner === 'Reflect'
            || callable.owner === 'ObjectConstructor' && ['keys', 'values', 'entries', 'getOwnPropertyNames', 'getOwnPropertySymbols', 'getOwnPropertyDescriptor', 'getOwnPropertyDescriptors', 'defineProperty', 'defineProperties', 'assign', 'create', 'hasOwn'].includes(callable.name)
            || callable.owner === 'Object' && ['hasOwnProperty', 'propertyIsEnumerable'].includes(callable.name)
            || callable.owner === 'JSON' && ['parse', 'stringify'].includes(callable.name);
          unsafe = reflection && (node.arguments.some(reflectiveArgument)
            || ts.isPropertyAccessExpression(node.expression) && relevant(node.expression.expression));
          // JSON.parse produces untyped data whose keys the compiler cannot bind.
          if (callable.owner === 'JSON' && callable.name === 'parse') unsafe = true;
        }
      } else if (ts.isBinaryExpression(node) && node.operatorToken.kind === ts.SyntaxKind.InKeyword) unsafe = relevant(node.right);
      else if (ts.isForInStatement(node)) unsafe = relevant(node.expression);
      else if (ts.isSpreadAssignment(node) || ts.isJsxSpreadAttribute(node)) unsafe = relevant(node.expression);
      else if (ts.isVariableDeclaration(node) && ts.isObjectBindingPattern(node.name)
        && node.name.elements.some(e => e.dotDotDotToken)) unsafe = relevant(node.initializer);
      if (unsafe) issue('Reflexão/enumeração/serialização de membros exige prova adicional de chaves em runtime', name, range(f.before, node.getStart(sf), node.end));
      ts.forEachChild(node, visit);
    };
    visit(sf);
  }
  if (output.issues.length) return;
  for (const [name, list] of originalTokens) {
    for (const edit of files.get(name).edits) {
      if (![...list.values()].some(t => t.start <= edit.start && edit.end <= t.end && (t.symbol || t.valueSymbol))) issue('Edição sem token de binding comprovado', name, edit.range);
    }
  }
  if (output.issues.length) return;
  const after = service(true);
  if (unresolved.size) throw new Error('Imports após rename fora do corpus');
  diagnostics(after, true);
  if (output.conflicts.length) { output.coverage = 'Rejected'; return; }
  if (output.issues.length) return;
  const updatedTokens = tokens(after, true);
  for (const [name, list] of originalTokens) {
    const f = files.get(name);
    for (const token of list.values()) {
      budget(); let start = forward(f, token.start), end = forward(f, token.end);
      const edit = f.edits.find(e => e.start === token.start && e.end === token.end);
      if (edit) { start = edit.afterStart + (edit.prefix || '').length; end = start + input.newName.length; }
      if (start === null || end === null) { issue('Edição muda estrutura interna de token não comprovada', name, range(f.before, token.start, token.end)); continue; }
      const changed = updatedTokens.get(name).get(`${start}:${end}`);
      let originalIdentity = token.symbol;
      if (edit?.prefix && token.valueSymbol) originalIdentity = token.valueSymbol;
      if (!changed || originalIdentity !== changed.symbol) conflict('Rename altera identidade de declaração/referência no compilador', name, range(f.before, token.start, token.end));
      if (token.valueSymbol && changed) {
        const assignment = changed.node.parent;
        if (ts.isPropertyAssignment(assignment)) {
          const key = symbolIdentity(after.checker.getSymbolAtLocation(assignment.name), after, true);
          const value = symbolIdentity(after.checker.getSymbolAtLocation(assignment.initializer), after, true);
          if (key !== token.symbol || value !== token.valueSymbol) conflict('Expansão de shorthand altera binding da chave ou do valor', name, range(f.before, token.start, token.end));
        } else if (changed.valueSymbol !== token.valueSymbol) conflict('Shorthand adquire outro binding de valor', name, range(f.before, token.start, token.end));
      }
      if (edit && !token.symbol) issue('Referência editada não possui identidade nominal comprovada', name, edit.range);
    }
  }
  output.coverage = output.conflicts.length ? 'Rejected' : output.issues.length ? 'Unsupported' : 'Verified';
  output.note = 'Proposta exata do language service, sintaxe, diagnósticos habilitados pela configuração real e identidades de declarações/referências conferidos em snapshots. JavaScript sem checkJs usa prova de vínculos, sem habilitar diagnósticos silenciosamente. Imports externos e referências dinâmicas não comprovadas bloqueiam; não prova efeitos externos ou reflexão em runtime.';
  before.service.dispose(); after.service.dispose();
}
try {
  const bytes = fs.readFileSync(0);
  if (bytes.length > 20000000) throw new Error('Entrada JSON excede limite');
  main(JSON.parse(bytes.toString('utf8')));
} catch (error) {
  issue(error instanceof Error ? error.message : String(error));
  output.coverage = output.conflicts.length ? 'Rejected' : 'Unsupported';
}
process.stdout.write(JSON.stringify(output));
