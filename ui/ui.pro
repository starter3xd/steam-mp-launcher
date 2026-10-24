QT += widgets network dbus
CONFIG += c++20
TEMPLATE = app
TARGET = steam-mp-launcher
DESTDIR = $$PWD

SOURCES += \
    main.cpp \
    appwindow.cpp \
    confirm.cpp \
    installer_dialog.cpp \
    trucksbook_dialog.cpp \
    openwith.cpp \
    ipc.cpp \
    backend.cpp \
    manager.cpp \
    picker.cpp \
    tray.cpp \
    theme.cpp \
    notify.cpp

HEADERS += \
    appwindow.h \
    confirm.h \
    installer_dialog.h \
    trucksbook_dialog.h \
    openwith.h \
    ipc.h \
    backend.h \
    manager.h \
    picker.h \
    tray.h \
    theme.h \
    i18n.h \
    notify.h

# Only the tray library: QSystemTrayIcon does not speak StatusNotifierItem in
# Qt 6.12, and the session tray is the only session UI on Wayland.
INCLUDEPATH += $$PWD/../backend/include \
    /usr/include/KF6/KStatusNotifierItem
LIBS += -L$$PWD/../backend/target/release -lmp_backend -lKF6StatusNotifierItem -lpthread -ldl -lm
PRE_TARGETDEPS += $$PWD/../backend/target/release/libmp_backend.a
QMAKE_CXXFLAGS += -Wall -Wextra

