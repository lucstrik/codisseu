// Independent compatibility adapter. Not the Postman sandbox implementation.
// All pm objects/functions are created inside the vm; no host object is injected.
'use strict';
const vm = require('node:vm');
let input = '';
process.stdin.setEncoding('utf8');
process.stdin.on('data', chunk => {
  input += chunk;
  if (input.length > 16 * 1024 * 1024) process.exit(2);
});
process.stdin.on('end', () => {
  try {
    const source = JSON.parse(input);
    const context = vm.createContext(Object.create(null), {
      codeGeneration: {strings: false, wasm: false},
      microtaskMode: 'afterEvaluate',
    });
    const script = new vm.Script(source);
    const result = script.runInContext(context, {timeout: 1000, breakOnSigint: true});
    if (typeof result !== 'string' || Buffer.byteLength(result) > 1024 * 1024) process.exit(3);
    process.stdout.write(result);
  } catch (error) {
    // Only fixed adapter codes cross the boundary; never echo arbitrary errors.
    process.exitCode = error?.message === 'CODISSEU_NO_ENVIRONMENT' ? 5
      : error?.message === 'CODISSEU_UNDEFINED_VARIABLE' ? 6 : 4;
  }
});
