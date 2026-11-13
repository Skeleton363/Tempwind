/* Renames wineserver's global symbols that ntdll's Unix part (with src/) also
 * defines, so both link into one module (docs/design/wine-unix.md, section 2,
 * gate 3). build.sh force-includes this file into the server's sources only.
 *
 * Each NAME becomes wineserver_NAME; gate 3 strips that prefix to recover the
 * server's names. Regenerate the collision list with `build.sh --census`.
 * Later packages append lines here, each with its reason.
 */
#ifndef TEMPWIND_RENAME_SERVER_H
#define TEMPWIND_RENAME_SERVER_H

/* Not a collision with ntdll: the server's main() would be the module's entry
 * point. The in-process server never runs it (section 7). */
#define main wineserver_main

/* The census of 2026-11-13 (wine-11.0, 00xx patches): 7 collisions. */
#define get_thread_context wineserver_get_thread_context   /* ptrace.c; ntdll: thread.c */
#define native_machine wineserver_native_machine           /* registry.c; ntdll: server.c */
#define server_start_time wineserver_server_start_time     /* request.c; ntdll: server.c */
#define set_thread_context wineserver_set_thread_context   /* ptrace.c; ntdll: thread.c */
#define supported_machines wineserver_supported_machines   /* registry.c; ntdll: server.c */
#define supported_machines_count wineserver_supported_machines_count /* registry.c; ntdll: server.c */
#define user_shared_data wineserver_user_shared_data       /* fd.c; ntdll: virtual.c */

#endif /* TEMPWIND_RENAME_SERVER_H */
