/* config.h for Wine 11.0's ntdll Unix part and wineserver, built with
 * Emscripten 6.0.12 (wasm64). Hand-curated in place of configure's result
 * (docs/design/wine-unix.md, section 2): a non-Linux POSIX profile.
 *
 * - Every HAVE_ defined here is true of the 6.0.12 sysroot: the header
 *   compiles, the function links (probed with emcc -m64 -sSHARED_MEMORY).
 * - Linux-only interfaces stay off even where the sysroot has a header or a
 *   symbol: asm/, linux/, scsi/, epoll, inotify, prctl, futexes, sysinfo,
 *   vfs.h, mntent, mtio, the syscall numbers, si_fd, ifr_hwaddr and the
 *   Linux-style gethostbyname_r.
 * - Everything configure would leave undefined here (the macOS, BSD and
 *   Solaris interfaces the sysroot lacks, the optional libraries, the SONAMEs)
 *   is simply absent.
 *
 * build.sh force-includes this file (-include) and writes the tree's
 * include/config.h as a forwarder, so `#include "config.h"` finds it too.
 * Later packages append lines here, each with its reason.
 */
#ifndef __WINE_CONFIG_H
#define __WINE_CONFIG_H

/* The package (configure.ac's AC_INIT). */
#define PACKAGE_BUGREPORT "wine-devel@winehq.org"
#define PACKAGE_NAME "Wine"
#define PACKAGE_STRING "Wine 11.0"
#define PACKAGE_TARNAME "wine"
#define PACKAGE_URL "https://www.winehq.org"
#define PACKAGE_VERSION "11.0"
#define EXEEXT ""

/* System extensions (AC_USE_SYSTEM_EXTENSIONS); musl needs _GNU_SOURCE for
 * pipe2, struct ucred and the like. off_t and time_t are 64-bit on wasm64. */
#ifndef _GNU_SOURCE
#define _GNU_SOURCE 1
#endif
#define _FILE_OFFSET_BITS 64
#define _TIME_BITS 64

/* Standard C and POSIX headers (AC_INCLUDES_DEFAULT). */
#define STDC_HEADERS 1
#define HAVE_INTTYPES_H 1
#define HAVE_STDINT_H 1
#define HAVE_STDIO_H 1
#define HAVE_STDLIB_H 1
#define HAVE_STRINGS_H 1
#define HAVE_STRING_H 1
#define HAVE_SYS_STAT_H 1
#define HAVE_SYS_TYPES_H 1
#define HAVE_UNISTD_H 1

/* Headers (configure.ac's AC_CHECK_HEADERS) that exist and are not
 * Linux-only. */
#define HAVE_ARPA_INET_H 1
#define HAVE_ARPA_NAMESER_H 1
#define HAVE_NETDB_H 1
#define HAVE_NETINET_IN_H 1
#define HAVE_NETINET_IN_SYSTM_H 1
#define HAVE_NETINET_TCP_H 1
#define HAVE_NETINET_IP_H 1
#define HAVE_NETINET_IP_ICMP_H 1
#define HAVE_NETINET_UDP_H 1
#define HAVE_NETINET_ICMP6_H 1
#define HAVE_NETINET_IF_ETHER_H 1
#define HAVE_NET_IF_H 1
#define HAVE_NET_IF_ARP_H 1
#define HAVE_NET_ROUTE_H 1
#define HAVE_IFADDRS_H 1
#define HAVE_PWD_H 1
#define HAVE_SCHED_H 1
#define HAVE_SYS_PARAM_H 1
#define HAVE_SYS_RESOURCE_H 1
#define HAVE_SYS_SIGNAL_H 1
#define HAVE_SYS_SOCKETVAR_H 1
#define HAVE_SYS_STATFS_H 1   /* struct statfs: fstatfs is general Unix; Wine's Linux uses stay under `linux` */
#define HAVE_SYS_STATVFS_H 1
#define HAVE_SYS_TIMES_H 1
#define HAVE_SYS_UCONTEXT_H 1
#define HAVE_SYS_UIO_H 1
#define HAVE_SYS_UN_H 1
#define HAVE_SYS_UTSNAME_H 1
#define HAVE_UTIME_H 1

/* Functions (AC_CHECK_FUNCS, AC_SEARCH_LIBS) that link. */
#define HAVE_CLOCK_GETTIME 1
#define HAVE_FSTATFS 1
#define HAVE_FUTIMENS 1
#define HAVE_GETADDRINFO 1
#define HAVE_GETIFADDRS 1
#define HAVE_PIPE2 1
#define HAVE_POSIX_FADVISE 1
#define HAVE_POSIX_FALLOCATE 1
#define HAVE_SCHED_YIELD 1
#define HAVE_SIGPROCMASK 1
#define HAVE_TCDRAIN 1

/* Members (AC_CHECK_MEMBERS). */
#define HAVE_STRUCT_STAT_ST_ATIM 1
#define HAVE_STRUCT_STAT_ST_CTIM 1
#define HAVE_STRUCT_STAT_ST_MTIM 1
#define HAVE_STRUCT_SOCKADDR_IN6_SIN6_SCOPE_ID 1
#define HAVE_STRUCT_IN6_PKTINFO_IPI6_ADDR 1

/* Builtins. */
#define HAVE___BUILTIN_POPCOUNT 1

#endif /* __WINE_CONFIG_H */
