# SPDX-License-Identifier: MIT
cmake_minimum_required(VERSION 3.20)
project(steam-mp-launcher VERSION 0.1.1 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)
set(CMAKE_INCLUDE_CURRENT_DIR ON)

find_package(Qt6 REQUIRED COMPONENTS Widgets Network DBus)
# Only the tray keeps a KF6 dependency: Qt's QSystemTrayIcon does not speak
# StatusNotifierItem (Wayland) here, and the tray is the whole session UI.
find_package(KF6StatusNotifierItem REQUIRED)
find_program(CARGO_EXECUTABLE cargo REQUIRED)

# KDE strongly recommends extra-cmake-modules/KDEInstallDirs; use it when
# available and fall back to GNUInstallDirs for a plain Qt build.
find_package(ECM QUIET NO_MODULE)
if(ECM_FOUND)
    set(CMAKE_MODULE_PATH ${ECM_MODULE_PATH} ${CMAKE_MODULE_PATH})
    include(KDEInstallDirs6)
else()
    include(GNUInstallDirs)
    set(KDE_INSTALL_BINDIR ${CMAKE_INSTALL_BINDIR})
    set(KDE_INSTALL_APPDIR ${CMAKE_INSTALL_DATADIR}/applications)
    set(KDE_INSTALL_METAINFODIR ${CMAKE_INSTALL_DATADIR}/metainfo)
    set(KDE_INSTALL_ICONDIR ${CMAKE_INSTALL_DATADIR}/icons)
endif()

# The Rust static library (and the mp-run Steam runner binary).
set(RUST_MANIFEST ${CMAKE_CURRENT_SOURCE_DIR}/backend/Cargo.toml)
set(RUST_LIB ${CMAKE_CURRENT_SOURCE_DIR}/backend/target/release/libmp_backend.a)
file(GLOB_RECURSE RUST_SOURCES CONFIGURE_DEPENDS ${CMAKE_CURRENT_SOURCE_DIR}/backend/src/*.rs)
add_custom_command(
    OUTPUT ${RUST_LIB}
    COMMAND ${CARGO_EXECUTABLE} build --release --manifest-path ${RUST_MANIFEST}
    DEPENDS ${RUST_SOURCES} ${RUST_MANIFEST} ${CMAKE_CURRENT_SOURCE_DIR}/backend/Cargo.lock
    WORKING_DIRECTORY ${CMAKE_CURRENT_SOURCE_DIR}
    COMMENT "Building the Rust backend"
    VERBATIM
)
add_custom_target(mp_backend ALL DEPENDS ${RUST_LIB})

set(UI_SOURCES
    ui/appwindow.cpp
    ui/backend.cpp
    ui/confirm.cpp
    ui/installer_dialog.cpp
    ui/ipc.cpp
    ui/main.cpp
    ui/manager.cpp
    ui/notify.cpp
    ui/openwith.cpp
    ui/picker.cpp
    ui/theme.cpp
    ui/tray.cpp
    ui/trucksbook_dialog.cpp
)
set(UI_HEADERS
    ui/appwindow.h
    ui/backend.h
    ui/confirm.h
    ui/installer_dialog.h
    ui/ipc.h
    ui/manager.h
    ui/notify.h
    ui/openwith.h
    ui/picker.h
    ui/theme.h
    ui/tray.h
    ui/trucksbook_dialog.h
)

add_executable(steam-mp-launcher ${UI_SOURCES} ${UI_HEADERS})
set_target_properties(steam-mp-launcher PROPERTIES
    RUNTIME_OUTPUT_DIRECTORY ${CMAKE_CURRENT_SOURCE_DIR}/ui
)
target_include_directories(steam-mp-launcher PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/backend/include
    ${CMAKE_CURRENT_SOURCE_DIR}/ui
)
target_link_libraries(steam-mp-launcher PRIVATE
    Qt6::Widgets Qt6::Network Qt6::DBus
    KF6::StatusNotifierItem
    ${RUST_LIB} dl pthread m
)
add_dependencies(steam-mp-launcher mp_backend)

include(CTest)
if(BUILD_TESTING)
    add_test(NAME rust-tests
        COMMAND ${CARGO_EXECUTABLE} test --manifest-path ${RUST_MANIFEST}
    )
endif()

install(TARGETS steam-mp-launcher RUNTIME DESTINATION ${KDE_INSTALL_BINDIR})
install(FILES
    dist/io.github.steam_mp_launcher.SteamMpLauncher.desktop
    dist/steam-mp-launcher-next-open.desktop
    DESTINATION ${KDE_INSTALL_APPDIR}
)
install(FILES dist/io.github.steam_mp_launcher.SteamMpLauncher.metainfo.xml
    DESTINATION ${KDE_INSTALL_METAINFODIR})
install(FILES dist/steam-mp-launcher.svg dist/steam-mp-launcher-symbolic.svg
    DESTINATION ${KDE_INSTALL_ICONDIR}/hicolor/scalable/apps)
