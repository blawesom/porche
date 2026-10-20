import ctypes, errno, os
libc = ctypes.CDLL(None, use_errno=True)
class P(ctypes.Structure):
    _fields_ = [("x", ctypes.c_uint64 * 16)]
p = P()
r = libc.syscall(425, 4, ctypes.byref(p))
if r < 0:
    e = ctypes.get_errno()
    print(f"io_uring_setup(nobody+params): DENIED ({errno.errorcode.get(e, e)})")
else:
    print(f"io_uring_setup(nobody+params): ALLOWED (fd {r})")
    os.close(r)
