/* Writes the 4096 bytes at 0x7ffe0000 (KUSER_SHARED_DATA) to the file named
 * on the command line. tests/wine/prepare.sh runs it under native Wine 11 to
 * make target/wine/kuser.bin, which the boot test compares against
 * (docs/design/wine-unix.md, section 10). */
#include <windows.h>
#include <stdio.h>

int main(int argc, char **argv) {
    if (argc != 2) {
        fprintf(stderr, "usage: kuser-dump OUT\n");
        return 2;
    }
    HANDLE f = CreateFileA(argv[1], GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
    if (f == INVALID_HANDLE_VALUE) {
        fprintf(stderr, "kuser-dump: cannot create %s (error %lu)\n", argv[1], GetLastError());
        return 1;
    }
    DWORD n = 0;
    BOOL ok = WriteFile(f, (const void *)(ULONG_PTR)0x7ffe0000, 4096, &n, NULL);
    CloseHandle(f);
    if (!ok || n != 4096) {
        fprintf(stderr, "kuser-dump: write failed (%lu bytes, error %lu)\n", n, GetLastError());
        return 1;
    }
    return 0;
}
