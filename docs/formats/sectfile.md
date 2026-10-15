# Section files (`.ini`, `.mat`, `.him`, `.mis`, text `.msh`, ...)

Status: **verified** from the runtime's section-file reader.

A line-oriented text format organized in sections. Files are read as 8-bit text (Windows-1252 in practice).

## Line processing
For each line, after removing the line terminator (`\n`, `\r\n` or `\r`):

1. Skip leading characters `<= 0x20`.
2. **Comments** (enabled by default; a reader can turn them off):
   scanning left to right, a tab becomes a space, and the line ends at the first
   - control character (`< 0x20`),
   - `#` or `;`,
   - `//` (the line ends before the first slash).
3. Trim trailing characters `<= 0x20`. Empty lines are ignored.
4. A line that starts with `[` and ends with `]`, and has at least one character in between, starts a new section.
   The name is the text between the brackets, verbatim (case-sensitive, inner spaces kept).
5. Any other line before the first section is ignored.
6. Otherwise the line is an **entry**:
   - the key is everything up to the first character `<= 0x20`,
   - the value is the rest with leading whitespace removed (it may be empty).

Keys are not unique. A section is an ordered list of `(key, value)` pairs, and section names may repeat too.
Lookups in the original take the first match.

Some section files are additionally encrypted with a per-file key. That filter is documented separately (TODO).

## Engine (native) reader differences
The native engine reads `.msh`, `.mat`, `.him` and `.sim` files with its own reader, which differs slightly from the
Java one above:
- Key lookups are **case-insensitive** for ASCII letters (`TfTestA` matches `tfTestA`).
- A key also ends at `=` (`key=value`).
- `#`, `;` and `//` start comments.

Section names that start with `_` (for example `[_Layer1]`) are simply never looked up, which effectively
disables them.
