//! PE32+ (x86-64) image reader: headers, sections, imports, exports,
//! relocations, exception data (`.pdata`), TLS and the COFF symbol table.
//! Read-only parsing, shared by the census (`testkit census`) and, from M2,
//! ahead-of-time translation of whole images. Built in Phase 1 M1 track B.
