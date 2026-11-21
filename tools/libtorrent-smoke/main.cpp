#define NOMINMAX
#include <Windows.h>
#include <DbgHelp.h>

#include <libtorrent/ip_filter.hpp>
#include <libtorrent/session.hpp>
#include <libtorrent/settings_pack.hpp>
#include <libtorrent/version.hpp>

#include <cstdint>
#include <iostream>
#include <string>
#include <utility>

namespace {

LONG CALLBACK write_crash_dump(EXCEPTION_POINTERS* exception) {
  if (exception == nullptr || exception->ExceptionRecord == nullptr ||
      exception->ExceptionRecord->ExceptionCode != EXCEPTION_ACCESS_VIOLATION) {
    return EXCEPTION_CONTINUE_SEARCH;
  }

  wchar_t directory[MAX_PATH * 4]{};
  const DWORD length = GetEnvironmentVariableW(
      L"SENZO_LIBUTORRENT_DUMP_DIR", directory,
      static_cast<DWORD>(std::size(directory)));
  if (length == 0 || length >= std::size(directory) - 32) {
    return EXCEPTION_CONTINUE_SEARCH;
  }

  std::wstring path(directory, length);
  if (!path.empty() && path.back() != L'\\' && path.back() != L'/') {
    path.push_back(L'\\');
  }
  path += L"libtorrent-cpp-smoke.dmp";

  HANDLE file = CreateFileW(path.c_str(), GENERIC_WRITE, 0, nullptr,
                            CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, nullptr);
  if (file != INVALID_HANDLE_VALUE) {
    MINIDUMP_EXCEPTION_INFORMATION details{};
    details.ThreadId = GetCurrentThreadId();
    details.ExceptionPointers = exception;
    details.ClientPointers = FALSE;
    MiniDumpWriteDump(GetCurrentProcess(), GetCurrentProcessId(), file,
                      static_cast<MINIDUMP_TYPE>(MiniDumpWithFullMemory |
                                                 MiniDumpWithThreadInfo),
                      &details, nullptr, nullptr);
    CloseHandle(file);
  }

  return EXCEPTION_CONTINUE_SEARCH;
}

} // namespace

int main() {
  AddVectoredExceptionHandler(1, write_crash_dump);

  std::cout << "libtorrent version: " << lt::version() << std::endl;
  std::cout << "sizeof(ip_filter): " << sizeof(lt::ip_filter)
            << ", sizeof(session_params): " << sizeof(lt::session_params)
            << std::endl;

  std::cout << "[1] Construct standalone ip_filter..." << std::endl;
  lt::ip_filter standalone_filter;
  std::cout << "[2] standalone ip_filter.empty() = "
            << standalone_filter.empty() << std::endl;

  std::cout << "[3] Construct session_params without plugins..." << std::endl;
  lt::settings_pack pack;
  lt::session_params params(std::move(pack), {});
  std::cout << "[4] session_params constructed at " << &params
            << ", ip_filter at " << &params.ip_filter << std::endl;

  std::cout << "[5] session_params.ip_filter.empty() = "
            << params.ip_filter.empty() << std::endl;

  std::cout << "[6] Construct libtorrent::session..." << std::endl;
  lt::session session(std::move(params));
  std::cout << "[7] Direct C++ libtorrent session startup succeeded."
            << std::endl;

  auto proxy = session.abort();
  (void)proxy;
  std::cout << "[8] Direct C++ libtorrent session abort requested."
            << std::endl;
  return 0;
}
