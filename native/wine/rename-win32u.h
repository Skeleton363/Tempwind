/* Renames win32u's global symbols that ntdll's Unix part (with src/) or the
 * server also defines, so all three link into one module
 * (docs/design/wine-unix.md, section 2, gate 3). build.sh force-includes this
 * file into win32u's Unix sources only. ntdll keeps its names: win32u calls
 * ntdll's exports by name, as win32u.so does through ntdll.so.
 *
 * Each NAME becomes win32u_NAME; gate 3 strips that prefix to recover
 * win32u's names. Regenerate the collision list with `build.sh --census`.
 */
#ifndef TEMPWIND_RENAME_WIN32U_H
#define TEMPWIND_RENAME_WIN32U_H

/* The census of 2026-11-23 (wine-11.0, patches through 0401): 12 collisions,
 * 1 with ntdll and 11 with the server. */
#define init_startup_info win32u_init_startup_info             /* window.c; ntdll: env.c */
#define alloc_user_handle win32u_alloc_user_handle             /* window.c; server: user.c */
#define destroy_thread_windows win32u_destroy_thread_windows   /* window.c; server: window.c */
#define free_user_handle win32u_free_user_handle               /* window.c; server: user.c */
#define get_virtual_screen_rect win32u_get_virtual_screen_rect /* sysparams.c; server: window.c */
#define get_window_thread win32u_get_window_thread             /* window.c; server: window.c */
#define is_desktop_class win32u_is_desktop_class               /* class.c; server: class.c */
#define is_message_class win32u_is_message_class               /* class.c; server: class.c */
#define is_window_visible win32u_is_window_visible             /* window.c; server: window.c */
#define mirror_region win32u_mirror_region                     /* region.c; server: region.c */
#define send_notify_message win32u_send_notify_message         /* message.c; server: queue.c */
#define shared_session win32u_shared_session                   /* winstation.c; server: mapping.c */

#endif /* TEMPWIND_RENAME_WIN32U_H */
