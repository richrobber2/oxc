import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";

return (async () => {
  const require = createRequire(import.meta.url);
  const helperPath = require.resolve("@oxc-project/runtime/helpers/asyncIterator");
  const asyncIterator = require(helperPath);
  const esm = await import(new URL("./esm/asyncIterator.js", pathToFileURL(helperPath)));
  for (const wrap of [asyncIterator, esm.default]) {
    const originalError = new Error("original");
    const abrupt = new Error("abrupt");
    const wrapped = (iterator) =>
      wrap({
        [Symbol.iterator]() {
          return iterator;
        },
      });

    for (const done of [undefined, null, 0, "", 1, "done", {}]) {
      const inner = {
        next() {},
        throw(error) {
          expect(this).toBe(inner);
          expect(error).toBe(originalError);
          return { value: Promise.resolve(1), done };
        },
      };
      await expect(wrapped(inner).throw(originalError)).resolves.toEqual({
        value: 1,
        done: !!done,
      });
    }

    // A missing throw method closes the iterator with no argument, then rejects TypeError.
    for (const missing of [undefined, null]) {
      let calls = 0;
      const inner = {
        next() {},
        throw: missing,
        return() {
          expect(this).toBe(inner);
          expect(arguments.length).toBe(0);
          calls++;
          return {
            get done() {
              throw abrupt;
            },
            get value() {
              throw abrupt;
            },
          };
        },
      };
      const result = wrapped(inner).throw(originalError);
      expect(result).toBeInstanceOf(Promise);
      await expect(result).rejects.toBeInstanceOf(TypeError);
      expect(calls).toBe(1);
    }
    await expect(wrapped({ next() {} }).throw(originalError)).rejects.toBeInstanceOf(TypeError);

    // Access, call, and iterator-result failures must become rejected promises.
    for (const inner of [
      {
        next() {},
        get throw() {
          throw abrupt;
        },
      },
      {
        next() {},
        throw() {
          throw abrupt;
        },
      },
      {
        next() {},
        throw() {
          return {
            get done() {
              throw abrupt;
            },
          };
        },
      },
      {
        next() {},
        throw() {
          return {
            done: true,
            get value() {
              throw abrupt;
            },
          };
        },
      },
      {
        next() {},
        get return() {
          throw abrupt;
        },
      },
      {
        next() {},
        return() {
          throw abrupt;
        },
      },
    ]) {
      const result = wrapped(inner).throw(originalError);
      expect(result).toBeInstanceOf(Promise);
      await expect(result).rejects.toBe(abrupt);
    }
    for (const inner of [
      { next() {}, throw: 1 },
      { next() {}, return: 1 },
      {
        next() {},
        return() {
          return 1;
        },
      },
      {
        next() {},
        throw() {
          return 1;
        },
      },
    ]) {
      const result = wrapped(inner).throw(originalError);
      expect(result).toBeInstanceOf(Promise);
      await expect(result).rejects.toBeInstanceOf(TypeError);
    }
  }
})();
