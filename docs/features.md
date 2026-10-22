# Features and limitations

The transpiler supports a useful subset of Rust syntax; it is not a Rust
compiler and does not guarantee equivalent behavior for every valid Rust
program. The table describes translation capabilities present in the current
implementation. The per-feature fixtures in `tests/fixtures/` are regression
examples, not a completeness claim.

| Area | Translation coverage |
| --- | --- |
| Items | Free functions, structs, enums, traits, impl blocks, constants, type aliases, inline modules, and `use` declarations |
| Types | Primitive types, strings, references, pointers, tuples, arrays, slices, functions, generics, and common standard-library containers |
| Expressions | Literals, calls, operators, field and index access, blocks, closures, casts, ranges, and common collection/runtime methods |
| Control flow | `if`, `match`, `while`, `loop`, `for`, `return`, `break`, and `continue` |
| Patterns | Identifier, tuple, struct, enum-variant, literal, range, reference, or-pattern, and slice patterns |
| Formatting | Rust formatting arguments and selected macros are translated to C++ equivalents or runtime helpers |
| Project files | The directory driver emits C++ header/source pairs, copies the runtime header, and can generate a basic CMake project file |

Items or expressions without a direct translation are represented by
`unsupported` comments in generated output where possible. Syntax errors are
reported; a successful translation only means the source was parsed and
processed, not that the output compiles or preserves Rust semantics.

Rust ownership and borrowing rules, lifetimes, async execution, trait
coherence, and the full standard library are not implemented as a Rust
semantic model. Some standard-library types and methods map to lightweight
runtime support or C++ standard-library equivalents, and edge cases may need
manual changes. Review generated C++ and test it with the intended compiler
and project dependencies before relying on it.
