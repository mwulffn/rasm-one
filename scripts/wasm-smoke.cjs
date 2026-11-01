// Runs the rasm-one JavaScript bindings under Node. See wasm-smoke.sh.
const assert = require("node:assert");
const { Project } = require(process.argv[2]);

const encode = (text) => new TextEncoder().encode(text);
const hex = (bytes) => Buffer.from(bytes).toString("hex");

// A program using a macro, an include and a local label.
const project = new Project();
project.add_file(
  "src/main.s",
  encode(
    'twice macro\n \\1\n \\1\n endm\n include "defs.i"\nstart: twice nop\n.l: move.w #value,d0\n bra .l\n',
  ),
);
project.add_file("lib/defs.i", encode("value equ $1234\n"));
project.add_include_dir("lib");

const started = performance.now();
const binary = project.assemble("src/main.s", "bin");
const elapsed = performance.now() - started;
assert.strictEqual(hex(binary.bytes()), "4e714e71303c123460fa");

project.set_optimise(false);
assert.strictEqual(hex(project.assemble("src/main.s", "bin").bytes()), "4e714e71303c12346000fffa");
project.set_optimise(true);

const linker = new Project();
linker.add_file("a.s", encode(" xref b\n bsr b\n rts\n"));
linker.add_file("b.s", encode(" xdef b\nb: rts\n"));
const linked = linker.assemble_and_link(["a.s", "b.s"]);
assert.strictEqual(JSON.parse(linked.report_json()).diagnostics.length, 0);
assert.ok(hex(linked.bytes()).startsWith("000003f3"));

const report = JSON.parse(binary.report_json());
assert.strictEqual(report.outputError, null);
assert.deepStrictEqual(report.diagnostics, []);
assert.deepStrictEqual(report.files, ["src/main.s", "lib/defs.i"]);
assert.strictEqual(report.symbols.find((s) => s.name === "start").value, 0);
assert.strictEqual(report.symbols.find((s) => s.name === "value").section, null);

// The first byte came from a macro body, called on line 6.
const first = report.lineTable[0];
assert.strictEqual(first.span.line, 2);
const expansion = report.expansions[first.expansion];
assert.strictEqual(expansion.kind, "macro");
assert.strictEqual(expansion.site.line, 6);

// The other output formats.
assert.strictEqual(hex(project.assemble("src/main.s", "hunkexe").bytes()).slice(0, 8), "000003f3");
assert.strictEqual(hex(project.assemble("src/main.s", "hunk").bytes()).slice(0, 8), "000003e7");
assert.match(binary.listing(), /4E71/);

// Errors come back as data, with locations.
const broken = new Project();
broken.add_file("bad.s", encode(" nop\n bogus\n"));
const failed = broken.assemble("bad.s", "bin");
assert.strictEqual(failed.bytes().length, 0);
const failure = JSON.parse(failed.report_json());
assert.strictEqual(failure.outputError, "assembly has errors");
assert.strictEqual(failure.diagnostics[0].code, "unknown-mnemonic");
assert.strictEqual(failure.diagnostics[0].span.line, 2);

// Macro recursion must be an error, not a blown WASM stack.
const recursive = new Project();
recursive.add_file("r.s", encode("m macro\n m\n endm\n m\n"));
const recursion = JSON.parse(recursive.assemble("r.s", "bin").report_json());
assert.ok(recursion.diagnostics.some((d) => d.code === "macro-depth"));

console.log(`WASM smoke test passed; assembled in ${elapsed.toFixed(2)} ms`);
