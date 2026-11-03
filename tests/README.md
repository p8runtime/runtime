# Manual PHP Examples

This folder mirrors small snippets lifted from the PHP manual so you can validate language features through `bun testrunner.js <category>`.

Current categories (each folder contains PHP snippets you can exercise):

- `arrays/`: literal arrays plus key/slice helpers and utility functions.
- `arrays/_pending/manual/`: auto-generated coverage of the array API (one file per helper); keep it pending until the runtime implements the missing functions.
- `constants/`: `const` and `define` checks tied to runtime metadata.
- `control_structures/`: branching and loop forms (if/else, switch, for/while/foreach, match, do/while, declaration examples).
- `date_time/`: `DateTime`/`DateTimeImmutable` usage, modification, and timezone formatting.
- `error_handling/`: exceptions, try/catch/finally flows.
- `filesystem/`: file helpers such as `file_put_contents`/`file_get_contents`.
- `functions/`: named functions, recursion, variadic helpers, and parameter variants.
- `generators/`: generator functions with `yield` and return values.
- `enums/`: unit and backed enums (cases, `from`/`tryFrom`).
- `magic_constants/`: `__FILE__` in an included file (`include_stub.php`).
- `math/`: numeric helpers (`pi`, `pow`, `max`).
- `math/_pending/manual/`: auto-generated coverage of the math helpers (one file per API); keep it pending until the interpreter implements the necessary functions.
- `namespaces/`: namespace declarations and `use` statements.
- `oop/`: class/method definitions (base classes, inheritance) and object behavior.
- `operators/`: arithmetic, bitwise, concatenation, null-coalescing, and spaceship operators (includes precedence).
- `string_functions/`: string helpers, heredoc/nowdoc literals, and formatting utilities.
- `string_functions/_pending/manual/`: auto-generated coverage of the entire string extension (one file per API); keep the `_pending` prefix until the runtime implements those builtins.
- `superglobals/`: server globals, CLI argv handling, and runtime inspection helpers.
- `templates/`: PHP embedded within HTML, using the alternative control-structure syntax.
- `traits/`: trait definitions and reuse patterns.
- `types/`: explicit casting between strings, ints, floats, and bools.
- `attributes/`: class-level attributes with reflection metadata.

Run `bun testrunner.js <category>` (e.g., `arrays`, `control_structures`, `operators`, etc.) to compare the selected folder against the official CLI output.
Pass `--skip=relative/path/to/fixture.php` (or comma separated values) to temporarily avoid a known failing script while keeping the rest of the suite green.

Paths inside directories whose names start with `_` (for example `tests/enums/_pending/serialization.php`) are excluded by default; keep unfinished/manual-only scripts there until the runtime supports the feature.

To refresh the string-function automations under `tests/string_functions/_pending/manual/`, run `node scripts/generate_string_tests.js` from the repo root. That script drops one PHP file per helper so you can keep the fixtures in sync if more functions get added.
To refresh the array-function automations under `tests/arrays/_pending/manual/`, run `node scripts/generate_array_tests.js` from the same location.
To refresh the math-function automations under `tests/math/_pending/manual/`, run `node scripts/generate_math_tests.js`; it behaves the same for the math helper set.
