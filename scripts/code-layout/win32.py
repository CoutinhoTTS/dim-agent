"""Small ctypes surface for native Windows training: working sets and ConPTY."""

import ctypes
from ctypes import wintypes
import hashlib
import os
import subprocess
import threading
import time

kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
psapi = ctypes.WinDLL("psapi", use_last_error=True)
ntdll = ctypes.WinDLL("ntdll", use_last_error=True)

PROCESS_QUERY_INFORMATION = 0x0400
PROCESS_SET_QUOTA = 0x0100
PROCESS_VM_READ = 0x0010
PROCESS_SUSPEND_RESUME = 0x0800
LIST_MODULES_ALL = 0x03
EXTENDED_STARTUPINFO_PRESENT = 0x00080000
CREATE_SUSPENDED = 0x00000004
CREATE_UNICODE_ENVIRONMENT = 0x00000400
PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE = 0x00020016
HPCON = ctypes.c_void_p


class SYSTEM_INFO(ctypes.Structure):
    _fields_ = [
        ("wProcessorArchitecture", wintypes.WORD),
        ("wReserved", wintypes.WORD),
        ("dwPageSize", wintypes.DWORD),
        ("lpMinimumApplicationAddress", ctypes.c_void_p),
        ("lpMaximumApplicationAddress", ctypes.c_void_p),
        ("dwActiveProcessorMask", ctypes.c_size_t),
        ("dwNumberOfProcessors", wintypes.DWORD),
        ("dwProcessorType", wintypes.DWORD),
        ("dwAllocationGranularity", wintypes.DWORD),
        ("wProcessorLevel", wintypes.WORD),
        ("wProcessorRevision", wintypes.WORD),
    ]


class PROCESS_MEMORY_COUNTERS_EX(ctypes.Structure):
    _fields_ = [
        ("cb", wintypes.DWORD),
        ("PageFaultCount", wintypes.DWORD),
        ("PeakWorkingSetSize", ctypes.c_size_t),
        ("WorkingSetSize", ctypes.c_size_t),
        ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
        ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
        ("PagefileUsage", ctypes.c_size_t),
        ("PeakPagefileUsage", ctypes.c_size_t),
        ("PrivateUsage", ctypes.c_size_t),
    ]


class MODULEINFO(ctypes.Structure):
    _fields_ = [
        ("lpBaseOfDll", ctypes.c_void_p),
        ("SizeOfImage", wintypes.DWORD),
        ("EntryPoint", ctypes.c_void_p),
    ]


class WORKING_SET_EX(ctypes.Structure):
    _fields_ = [("VirtualAddress", ctypes.c_void_p), ("Flags", ctypes.c_size_t)]


class FILETIME(ctypes.Structure):
    _fields_ = [("dwLowDateTime", wintypes.DWORD), ("dwHighDateTime", wintypes.DWORD)]


class STARTUPINFOW(ctypes.Structure):
    _fields_ = [
        ("cb", wintypes.DWORD),
        ("lpReserved", wintypes.LPWSTR),
        ("lpDesktop", wintypes.LPWSTR),
        ("lpTitle", wintypes.LPWSTR),
        ("dwX", wintypes.DWORD),
        ("dwY", wintypes.DWORD),
        ("dwXSize", wintypes.DWORD),
        ("dwYSize", wintypes.DWORD),
        ("dwXCountChars", wintypes.DWORD),
        ("dwYCountChars", wintypes.DWORD),
        ("dwFillAttribute", wintypes.DWORD),
        ("dwFlags", wintypes.DWORD),
        ("wShowWindow", wintypes.WORD),
        ("cbReserved2", wintypes.WORD),
        ("lpReserved2", ctypes.c_void_p),
        ("hStdInput", wintypes.HANDLE),
        ("hStdOutput", wintypes.HANDLE),
        ("hStdError", wintypes.HANDLE),
    ]


class STARTUPINFOEXW(ctypes.Structure):
    _fields_ = [("StartupInfo", STARTUPINFOW), ("lpAttributeList", ctypes.c_void_p)]


class PROCESS_INFORMATION(ctypes.Structure):
    _fields_ = [
        ("hProcess", wintypes.HANDLE),
        ("hThread", wintypes.HANDLE),
        ("dwProcessId", wintypes.DWORD),
        ("dwThreadId", wintypes.DWORD),
    ]


kernel32.GetSystemInfo.argtypes = [ctypes.POINTER(SYSTEM_INFO)]
kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
kernel32.OpenProcess.restype = wintypes.HANDLE
kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
psapi.QueryWorkingSetEx.argtypes = [wintypes.HANDLE, ctypes.c_void_p, wintypes.DWORD]
psapi.QueryWorkingSetEx.restype = wintypes.BOOL
psapi.GetProcessMemoryInfo.argtypes = [
    wintypes.HANDLE, ctypes.POINTER(PROCESS_MEMORY_COUNTERS_EX), wintypes.DWORD
]
psapi.GetProcessMemoryInfo.restype = wintypes.BOOL
psapi.EnumProcessModulesEx.argtypes = [
    wintypes.HANDLE, ctypes.c_void_p, wintypes.DWORD,
    ctypes.POINTER(wintypes.DWORD), wintypes.DWORD,
]
psapi.EnumProcessModulesEx.restype = wintypes.BOOL
psapi.GetModuleInformation.argtypes = [
    wintypes.HANDLE, wintypes.HMODULE, ctypes.POINTER(MODULEINFO), wintypes.DWORD
]
psapi.GetModuleInformation.restype = wintypes.BOOL
psapi.GetModuleBaseNameW.argtypes = [
    wintypes.HANDLE, wintypes.HMODULE, wintypes.LPWSTR, wintypes.DWORD
]
psapi.GetModuleBaseNameW.restype = wintypes.DWORD
kernel32.K32EmptyWorkingSet.argtypes = [wintypes.HANDLE]
kernel32.K32EmptyWorkingSet.restype = wintypes.BOOL
kernel32.GetProcessTimes.argtypes = [
    wintypes.HANDLE, ctypes.POINTER(FILETIME), ctypes.POINTER(FILETIME),
    ctypes.POINTER(FILETIME), ctypes.POINTER(FILETIME),
]
kernel32.GetProcessTimes.restype = wintypes.BOOL
ntdll.NtResumeProcess.argtypes = [wintypes.HANDLE]
ntdll.NtResumeProcess.restype = ctypes.c_long
ntdll.NtSuspendProcess.argtypes = [wintypes.HANDLE]
ntdll.NtSuspendProcess.restype = ctypes.c_long


def page_size():
    info = SYSTEM_INFO()
    kernel32.GetSystemInfo(ctypes.byref(info))
    return info.dwPageSize


def open_process(pid, access=PROCESS_QUERY_INFORMATION | PROCESS_SET_QUOTA | PROCESS_VM_READ | PROCESS_SUSPEND_RESUME):
    handle = kernel32.OpenProcess(access, False, pid)
    if not handle:
        raise ctypes.WinError(ctypes.get_last_error())
    return handle


def close(handle):
    if handle:
        kernel32.CloseHandle(handle)


def empty_working_set(handle):
    if not kernel32.K32EmptyWorkingSet(handle):
        raise ctypes.WinError(ctypes.get_last_error())


def resume(handle):
    status = ntdll.NtResumeProcess(handle)
    if status:
        raise OSError(f"NtResumeProcess failed: 0x{status & 0xFFFFFFFF:08x}")


def suspend(handle):
    status = ntdll.NtSuspendProcess(handle)
    if status:
        raise OSError(f"NtSuspendProcess failed: 0x{status & 0xFFFFFFFF:08x}")


def memory(handle):
    counters = PROCESS_MEMORY_COUNTERS_EX()
    counters.cb = ctypes.sizeof(counters)
    if not psapi.GetProcessMemoryInfo(handle, ctypes.byref(counters), counters.cb):
        raise ctypes.WinError(ctypes.get_last_error())
    return {
        "workingSet": counters.WorkingSetSize,
        "peakWorkingSet": counters.PeakWorkingSetSize,
        "private": counters.PrivateUsage,
        "pagefile": counters.PagefileUsage,
    }


def cpu(handle):
    created, exited, kernel, user = FILETIME(), FILETIME(), FILETIME(), FILETIME()
    if not kernel32.GetProcessTimes(
        handle, ctypes.byref(created), ctypes.byref(exited), ctypes.byref(kernel),
        ctypes.byref(user)
    ):
        raise ctypes.WinError(ctypes.get_last_error())

    def seconds(value):
        ticks = (value.dwHighDateTime << 32) | value.dwLowDateTime
        return ticks / 10_000_000

    return {"user": seconds(user), "kernel": seconds(kernel)}


def module(handle):
    needed = wintypes.DWORD()
    if not psapi.EnumProcessModulesEx(handle, None, 0, ctypes.byref(needed), LIST_MODULES_ALL):
        raise ctypes.WinError(ctypes.get_last_error())
    count = needed.value // ctypes.sizeof(wintypes.HMODULE)
    modules = (wintypes.HMODULE * count)()
    if not psapi.EnumProcessModulesEx(
        handle, ctypes.cast(modules, ctypes.c_void_p), ctypes.sizeof(modules),
        ctypes.byref(needed), LIST_MODULES_ALL
    ):
        raise ctypes.WinError(ctypes.get_last_error())
    info = MODULEINFO()
    if not psapi.GetModuleInformation(handle, modules[0], ctypes.byref(info), ctypes.sizeof(info)):
        raise ctypes.WinError(ctypes.get_last_error())
    name = ctypes.create_unicode_buffer(32768)
    psapi.GetModuleBaseNameW(handle, modules[0], name, len(name))
    return info.lpBaseOfDll, info.SizeOfImage, name.value


def working_set_pages(handle, base, rva, size):
    step = page_size()
    count = (size + step - 1) // step
    entries = (WORKING_SET_EX * count)()
    for index in range(count):
        entries[index].VirtualAddress = base + rva + index * step
    if not psapi.QueryWorkingSetEx(
        handle, ctypes.cast(entries, ctypes.c_void_p), ctypes.sizeof(entries)
    ):
        raise ctypes.WinError(ctypes.get_last_error())
    return step, [index for index in range(count) if entries[index].Flags & 1]


class COORD(ctypes.Structure):
    _fields_ = [("X", wintypes.SHORT), ("Y", wintypes.SHORT)]


class SECURITY_ATTRIBUTES(ctypes.Structure):
    _fields_ = [
        ("nLength", wintypes.DWORD),
        ("lpSecurityDescriptor", ctypes.c_void_p),
        ("bInheritHandle", wintypes.BOOL),
    ]


kernel32.CreatePseudoConsole.argtypes = [
    COORD, wintypes.HANDLE, wintypes.HANDLE, wintypes.DWORD, ctypes.POINTER(HPCON)
]
kernel32.CreatePseudoConsole.restype = ctypes.c_long
kernel32.ClosePseudoConsole.argtypes = [HPCON]
kernel32.InitializeProcThreadAttributeList.argtypes = [
    ctypes.c_void_p, wintypes.DWORD, wintypes.DWORD, ctypes.POINTER(ctypes.c_size_t)
]
kernel32.InitializeProcThreadAttributeList.restype = wintypes.BOOL
kernel32.UpdateProcThreadAttribute.argtypes = [
    ctypes.c_void_p, wintypes.DWORD, ctypes.c_size_t, ctypes.c_void_p,
    ctypes.c_size_t, ctypes.c_void_p, ctypes.c_void_p,
]
kernel32.UpdateProcThreadAttribute.restype = wintypes.BOOL
kernel32.DeleteProcThreadAttributeList.argtypes = [ctypes.c_void_p]
kernel32.CreateProcessW.argtypes = [
    wintypes.LPCWSTR, wintypes.LPWSTR, ctypes.c_void_p, ctypes.c_void_p,
    wintypes.BOOL, wintypes.DWORD, ctypes.c_void_p, wintypes.LPCWSTR,
    ctypes.POINTER(STARTUPINFOEXW), ctypes.POINTER(PROCESS_INFORMATION),
]
kernel32.CreateProcessW.restype = wintypes.BOOL
kernel32.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
kernel32.WaitForSingleObject.restype = wintypes.DWORD
kernel32.GetExitCodeProcess.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
kernel32.GetExitCodeProcess.restype = wintypes.BOOL
kernel32.TerminateProcess.argtypes = [wintypes.HANDLE, wintypes.UINT]
kernel32.TerminateProcess.restype = wintypes.BOOL


def _environment_block(environment):
    entries = sorted((f"{key}={value}" for key, value in environment.items()), key=str.upper)
    return ctypes.create_unicode_buffer("\0".join(entries) + "\0\0")


class Console:
    """A ConPTY child with a synchronous write side and a reader thread."""

    def __init__(self, arguments, cwd, environment, columns=120, rows=30,
                 suspended=True, inherit_environment=False):
        import msvcrt
        self._child_in, self._master_in = os.pipe()
        self._master_out, self._child_out = os.pipe()
        self._console = HPCON()
        status = kernel32.CreatePseudoConsole(
            COORD(columns, rows),
            ctypes.c_void_p(msvcrt.get_osfhandle(self._child_in)),
            ctypes.c_void_p(msvcrt.get_osfhandle(self._child_out)),
            0,
            ctypes.byref(self._console),
        )
        if status < 0:
            self.close()
            raise OSError(f"CreatePseudoConsole failed: 0x{status & 0xFFFFFFFF:08x}")

        size = ctypes.c_size_t()
        kernel32.InitializeProcThreadAttributeList(None, 1, 0, ctypes.byref(size))
        self._attributes = ctypes.create_string_buffer(size.value)
        attributes = ctypes.cast(self._attributes, ctypes.c_void_p)
        if not kernel32.InitializeProcThreadAttributeList(
            attributes, 1, 0, ctypes.byref(size)
        ):
            self.close()
            raise ctypes.WinError(ctypes.get_last_error())
        if not kernel32.UpdateProcThreadAttribute(
            attributes, 0, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
            self._console, ctypes.sizeof(HPCON), None, None,
        ):
            self.close()
            raise ctypes.WinError(ctypes.get_last_error())

        startup = STARTUPINFOEXW()
        startup.StartupInfo.cb = ctypes.sizeof(STARTUPINFOEXW)
        startup.lpAttributeList = ctypes.cast(self._attributes, ctypes.c_void_p)
        process = PROCESS_INFORMATION()
        command = ctypes.create_unicode_buffer(subprocess.list2cmdline([str(item) for item in arguments]))
        flags = EXTENDED_STARTUPINFO_PRESENT
        if suspended:
            flags |= CREATE_SUSPENDED
        environment_block = None
        if not inherit_environment:
            flags |= CREATE_UNICODE_ENVIRONMENT
            environment_block = ctypes.cast(
                _environment_block(environment), ctypes.c_void_p)
        if not kernel32.CreateProcessW(
            None, command, None, None, False, flags, environment_block,
            str(cwd), ctypes.byref(startup), ctypes.byref(process),
        ):
            self.close()
            raise ctypes.WinError(ctypes.get_last_error())
        self._process = process.hProcess
        self._thread = process.hThread
        self.pid = process.dwProcessId
        kernel32.CloseHandle(self._thread)
        self._thread = None
        self._buffer = bytearray()
        self.error = None
        self._lock = threading.Lock()
        self._stop = threading.Event()
        self._reader = threading.Thread(target=self._read, daemon=True)
        self._reader.start()

    def _read(self):
        while not self._stop.is_set():
            try:
                chunk = os.read(self._master_out, 65536)
            except OSError as error:
                self.error = error
                break
            if not chunk:
                break
            with self._lock:
                self._buffer.extend(chunk)

    def take(self):
        with self._lock:
            data = bytes(self._buffer)
            self._buffer.clear()
        return data

    def write(self, data):
        os.write(self._master_in, data)

    def trim(self):
        empty_working_set(self._process)
        resume(self._process)

    def resume(self):
        resume(self._process)

    def poll(self):
        result = kernel32.WaitForSingleObject(self._process, 0)
        if result == 0x102:
            return None
        if result != 0:
            raise ctypes.WinError(ctypes.get_last_error())
        code = wintypes.DWORD()
        if not kernel32.GetExitCodeProcess(self._process, ctypes.byref(code)):
            raise ctypes.WinError(ctypes.get_last_error())
        return code.value

    def wait(self, timeout=30):
        result = kernel32.WaitForSingleObject(self._process, int(timeout * 1000))
        if result == 0x102:
            raise TimeoutError("ConPTY child did not exit")
        if result != 0:
            raise ctypes.WinError(ctypes.get_last_error())
        code = wintypes.DWORD()
        if not kernel32.GetExitCodeProcess(self._process, ctypes.byref(code)):
            raise ctypes.WinError(ctypes.get_last_error())
        return code.value

    def terminate(self):
        if self._process:
            kernel32.TerminateProcess(self._process, 1)
            kernel32.WaitForSingleObject(self._process, 10_000)

    def close(self):
        self._stop.set()
        if getattr(self, "_reader", None):
            self._reader.join(timeout=2)
        if getattr(self, "_console", None) and self._console:
            kernel32.ClosePseudoConsole(self._console)
            self._console = None
        if getattr(self, "_process", None):
            kernel32.CloseHandle(self._process)
            self._process = None
        for name in ("_master_in", "_child_in", "_child_out", "_master_out"):
            descriptor = getattr(self, name, None)
            if descriptor is not None:
                try:
                    os.close(descriptor)
                except OSError:
                    pass
                setattr(self, name, None)


FILE_SHARE_READ = 0x00000001
FILE_SHARE_WRITE = 0x00000002
FILE_SHARE_DELETE = 0x00000004
FILE_FLAG_BACKUP_SEMANTICS = 0x02000000
GENERIC_READ = 0x80000000
GENERIC_WRITE = 0x40000000
OPEN_EXISTING = 3
INVALID_HANDLE_VALUE = ctypes.c_void_p(-1).value
kernel32.CreateFileW.argtypes = [
    wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, ctypes.c_void_p,
    wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE,
]
kernel32.CreateFileW.restype = wintypes.HANDLE
kernel32.GetFinalPathNameByHandleW.argtypes = [
    wintypes.HANDLE, wintypes.LPWSTR, wintypes.DWORD, wintypes.DWORD
]
kernel32.GetFinalPathNameByHandleW.restype = wintypes.DWORD


def canonical_path(path):
    handle = kernel32.CreateFileW(
        str(path), 0, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        None, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, None,
    )
    if not handle or handle == INVALID_HANDLE_VALUE:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        buffer = ctypes.create_unicode_buffer(32768)
        length = kernel32.GetFinalPathNameByHandleW(handle, buffer, len(buffer), 0)
        if not length or length >= len(buffer):
            raise ctypes.WinError(ctypes.get_last_error())
        return buffer.value
    finally:
        kernel32.CloseHandle(handle)


def default_pipe(native_home):
    """The endpoint the CLI derives from the canonical native home."""
    canonical = canonical_path(native_home)
    key = hashlib.sha256(f"dimcode:{canonical}".encode()).hexdigest()
    return rf"\\.\pipe\dim-rust-{key[:32]}"


def open_pipe(name, timeout=10):
    import msvcrt
    import time
    deadline = time.monotonic() + timeout
    while True:
        handle = kernel32.CreateFileW(
            name, GENERIC_READ | GENERIC_WRITE, 0, None, OPEN_EXISTING, 0, None
        )
        if handle not in (None, 0, INVALID_HANDLE_VALUE):
            break
        error = ctypes.get_last_error()
        if error not in (2, 231) or time.monotonic() >= deadline:
            raise ctypes.WinError(error)
        time.sleep(0.02)
    descriptor = msvcrt.open_osfhandle(handle, os.O_RDWR | os.O_BINARY)
    if descriptor < 0:
        kernel32.CloseHandle(handle)
        raise OSError("cannot adopt the named-pipe handle")
    return descriptor


def wait_for_exit(handle, timeout=30):
    result = kernel32.WaitForSingleObject(handle, int(timeout * 1000))
    if result == 0x102:
        raise TimeoutError("process did not exit")
    if result != 0:
        raise ctypes.WinError(ctypes.get_last_error())


def exit_code(handle):
    code = wintypes.DWORD()
    if not kernel32.GetExitCodeProcess(handle, ctypes.byref(code)):
        raise ctypes.WinError(ctypes.get_last_error())
    return code.value
