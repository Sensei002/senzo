cmake_minimum_required(VERSION 3.20)
project(senzo_libtorrent_smoke LANGUAGES CXX)

find_package(LibtorrentRasterbar CONFIG REQUIRED)

add_executable(libtorrent-smoke main.cpp)
target_compile_features(libtorrent-smoke PRIVATE cxx_std_17)
target_link_libraries(libtorrent-smoke PRIVATE LibtorrentRasterbar::torrent-rasterbar)
