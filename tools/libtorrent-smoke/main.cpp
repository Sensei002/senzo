#define NOMINMAX
#include <Windows.h>
#include <DbgHelp.h>

#include <libtorrent/ip_filter.hpp>
#include <libtorrent/session.hpp>
#include <libtorrent/settings_pack.hpp>
#include <libtorrent/version.hpp>

#include <array>
#include <cstdint>
#include <iomanip>
#include <iostream>
#include <set>
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

  std::cout << "[0] Verify the runner's native std::set..." << std::endl;
  std::set<int> native_set;
  native_set.insert(1);
  std::cout << "[0a] Native std::set is valid." << std::endl;

  std::cout << "[0b] Test plain std::set<std::array<uint8_t, 16>>..." << std::endl;
  std::set<std::array<std::uint8_t, 16>> plain_v6_set;
  plain_v6_set.insert({});
  const std::array<std::uint8_t, 16> zero_v6{};
  std::cout << "[0c] Plain 16-byte array set comparison = "
            << (*plain_v6_set.begin() == zero_v6) << std::endl;

  std::cout << "[0d] Construct standalone libtorrent IPv6 filter..." << std::endl;
  lt::aux::filter_impl<std::array<std::uint8_t, 16>> standalone_v6_filter;
  std::cout << "[0e] Standalone libtorrent IPv6 filter constructed." << std::endl;
  std::cout << "[0f] Standalone libtorrent IPv6 filter.empty() = "
            << standalone_v6_filter.empty() << std::endl;

  std::cout << "[1] Construct standalone ip_filter..." << std::endl;
  lt::ip_filter standalone_filter;
  std::cout << "[1a] Standalone ip_filter constructed." << std::endl;

  const auto* filter_words = reinterpret_cast<const std::uintptr_t*>(&standalone_filter);
  std::cout << "[1b] ip_filter raw words:";
  for (int i = 0; i < 4; ++i) std::cout << " 0x" << std::hex << filter_words[i];
  std::cout << std::dec << std::endl;

  struct TreeHeader {
    std::uintptr_t left;
    std::uintptr_t parent;
    std::uintptr_t right;
    std::uint8_t color;
    std::uint8_t is_nil;
  };
  const auto head_address = filter_words[2];
  std::cout << "[1c] IPv6 set head address = 0x" << std::hex << head_address
            << std::dec << std::endl;
  if (head_address >= 0x10000 && head_address < 0x0000800000000000ULL) {
    const auto* head = reinterpret_cast<const TreeHeader*>(head_address);
    std::cout << "[1d] IPv6 tree head left/parent/right = 0x" << std::hex
              << head->left << " 0x" << head->parent << " 0x" << head->right
              << std::dec << " color=" << unsigned(head->color)
              << " is_nil=" << unsigned(head->is_nil) << std::endl;
  }

  std::cout << "[2] Call standalone ip_filter.empty()..." << std::endl;
  const bool standalone_empty = standalone_filter.empty();
  std::cout << "[2a] standalone ip_filter.empty() = "
            << standalone_empty << std::endl;

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
