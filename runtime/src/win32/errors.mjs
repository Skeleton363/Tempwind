// Win32 error codes (winerror.h, checked against Wine 11 include/winerror.h)
// that the shim sets.
export const ERROR_SUCCESS = 0;
export const ERROR_INVALID_FUNCTION = 1;
export const ERROR_FILE_NOT_FOUND = 2;
export const ERROR_PATH_NOT_FOUND = 3;
export const ERROR_ACCESS_DENIED = 5;
export const ERROR_INVALID_HANDLE = 6;
export const ERROR_NOT_ENOUGH_MEMORY = 8;
export const ERROR_INVALID_DATA = 13;
export const ERROR_NO_MORE_FILES = 18;
export const ERROR_SHARING_VIOLATION = 32;
export const ERROR_HANDLE_EOF = 38;
export const ERROR_NOT_SUPPORTED = 50;
export const ERROR_FILE_EXISTS = 80;
export const ERROR_INVALID_PARAMETER = 87;
export const ERROR_INSUFFICIENT_BUFFER = 122;
export const ERROR_INVALID_NAME = 123;
export const ERROR_MOD_NOT_FOUND = 126;
export const ERROR_PROC_NOT_FOUND = 127;
export const ERROR_NEGATIVE_SEEK = 131;
export const ERROR_DIR_NOT_EMPTY = 145;
export const ERROR_ALREADY_EXISTS = 183;
export const ERROR_ENVVAR_NOT_FOUND = 203;
export const ERROR_MORE_DATA = 234;
export const ERROR_DIRECTORY = 267;
export const ERROR_TOO_MANY_POSTS = 298;
export const ERROR_MR_MID_NOT_FOUND = 317;
export const ERROR_INVALID_ADDRESS = 487;
export const ERROR_NOT_ALL_ASSIGNED = 1300;

/** A Win32 failure: the API sets `code` as the last error and returns its failure value. */
export class Win32Error extends Error {
  constructor(code, message = `Win32 error ${code}`) {
    super(message);
    this.code = code;
  }
}
