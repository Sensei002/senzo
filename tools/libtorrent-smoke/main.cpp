#include <libtorrent/session.hpp>
#include <libtorrent/settings_pack.hpp>
#include <libtorrent/version.hpp>

#include <iostream>
#include <utility>

int main() {
  std::cout << "libtorrent version: " << lt::version() << std::endl;

  lt::settings_pack pack;
  pack.set_str(lt::settings_pack::user_agent, "stream-server/1.0");
  pack.set_bool(lt::settings_pack::enable_dht, true);
  pack.set_bool(lt::settings_pack::enable_upnp, true);
  pack.set_bool(lt::settings_pack::enable_natpmp, true);
  pack.set_bool(lt::settings_pack::announce_to_all_trackers, true);
  pack.set_bool(lt::settings_pack::announce_to_all_tiers, true);

  std::cout << "Constructing libtorrent::session from session_params..." << std::endl;
  lt::session_params params(std::move(pack));
  lt::session session(std::move(params));
  std::cout << "Direct C++ libtorrent session startup succeeded." << std::endl;

  // Keep the proxy alive until the session's worker threads have stopped.
  auto proxy = session.abort();
  (void)proxy;
  std::cout << "Direct C++ libtorrent session shutdown succeeded." << std::endl;
  return 0;
}
