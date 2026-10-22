const assert = require("node:assert/strict");
const { test } = require("node:test");
const { pathToFileURL } = require("node:url");
const { resolve } = require("node:path");

for (const format of ["commonjs", "esm"]) {
  test(`${format}: async-from-sync throw forwards to the iterator's throw method`, async () => {
    const asyncIterator = format === "commonjs"
      ? require("../src/helpers/asyncIterator.js")
      : (await import(pathToFileURL(resolve(__dirname, "../src/helpers/esm/asyncIterator.js")))).default;
    for (const hasReturn of [false, true]) {
      const error = new Error("TEST");
      const calls = [];
      const inner = {
        next() { return { value: "a", done: false }; },
        throw(value) {
          assert.equal(this, inner);
          assert.equal(value, error);
          calls.push("throw");
          return { value: Promise.resolve("handled"), done: true };
        },
      };
      if (hasReturn) inner.return = () => { throw new Error("return must not be called"); };
      const iterator = asyncIterator({ [Symbol.iterator]() { return inner; } });
      assert.deepEqual(await iterator.next(), { value: "a", done: false });
      assert.deepEqual(await iterator.throw(error), { value: "handled", done: true });
      assert.deepEqual(calls, ["throw"]);
    }
  });
}
