#include "appwindow.h"
#include "confirm.h"
#include "installer_dialog.h"
#include "trucksbook_dialog.h"
#include "ipc.h"
#include "tray.h"
#include "backend.h"
#include "theme.h"

#include <QApplication>
#include <QGuiApplication>
#include "i18n.h"
#include <QCommandLineParser>
#include <QTextStream>
#include <QTimer>

int main(int argc, char** argv) {
    QApplication app(argc, argv);
    QApplication::setApplicationName("steam-mp-launcher");
    QApplication::setApplicationDisplayName("Steam MP Launcher");
    // Before any widget: dark mode and the icon theme have to be in place for
    // desktops that do not expose them to plain Qt (GNOME, the bundle).
    theme::applySystemTheme();
    QGuiApplication::setDesktopFileName(QStringLiteral("io.github.steam_mp_launcher.SteamMpLauncher"));
    // Window icon for the picker, manager and every dialog (the desktop file
    // covers the taskbar, this covers Alt+Tab and bare X11/Wayland runs).
    QApplication::setWindowIcon(theme::appIcon());

    QCommandLineParser parser;
    parser.setApplicationDescription("Steam MP Launcher - C++ UI / Rust backend");
    parser.addHelpOption();
    const QCommandLineOption manage("manage", "Open the manager instead of the picker.");
    const QCommandLineOption appid("appid", "Steam appid.", "appid");
    const QCommandLineOption shot("shot", "Grab the window to a PNG and exit.", "path");
    const QCommandLineOption page("page", "Manager page: overview, launchers, extras, proton, settings, logs.", "name");
    const QCommandLineOption launch("launch", "Launch this .exe immediately (smoke test).", "path");
    const QCommandLineOption pick("pick", "Helper mode: print the chosen launcher and exit.");
    const QCommandLineOption confirm("confirm", "Countdown before a remembered choice.");
    const QCommandLineOption label("label", "Launcher label for --confirm.", "label");
    const QCommandLineOption gameName("game-name", "Game name for --confirm.", "name");
    const QCommandLineOption seconds("seconds", "Countdown seconds for --confirm.", "seconds", "3");
    const QCommandLineOption log("log", "Run log for --tray.", "path");
    const QCommandLineOption parentPid("parent-pid", "Runner pid for --tray.", "pid");
    const QCommandLineOption tray("tray", "Show the session tray icon.");
    const QCommandLineOption open("open", "Open the add/run card for a .exe.", "file");
    const QCommandLineOption install("install", "Open the installer (truckersmp/trucksbook).", "kind");
    const QCommandLineOption themeInfo("theme-info", "Print the detected desktop theme and exit.");
    parser.addOptions({manage, appid, shot, page, launch, pick, confirm, label, gameName, seconds, log, parentPid, tray, open, install, themeInfo});
    parser.process(app);

    if (parser.isSet(themeInfo)) {
        QTextStream(stdout) << theme::systemThemeReport();
        return 0;
    }

    if (parser.isSet(confirm)) {
        ConfirmWindow card(parser.value(label), parser.value(gameName),
                           parser.value(seconds).toInt());
        return card.exec();
    }

    // Hand a second picker/manager request to the window that is already open.
    const bool pickMode = parser.isSet(pick);
    const bool plainPick = !pickMode && !parser.isSet(manage) && !parser.isSet(open) &&
                           !parser.isSet(launch) && !parser.isSet(tray) &&
                           !parser.isSet(install);
    if (!parser.isSet(shot) && !parser.isSet(page) &&
        (pickMode || parser.isSet(manage) || plainPick)) {
        const QJsonObject args = pickMode ? QJsonObject{{"appid", parser.value(appid)}} : QJsonObject{};
        QString value;
        int rc = 0;
        bool sidecars = true;
        const int timeout = pickMode ? 3600000 : 700;
        if (ipc::handoff(pickMode ? "pick" : "manage", args, &value, &rc, timeout, &sidecars)) {
            if (pickMode) {
                if (rc == 0 && !value.isEmpty()) {
                    QTextStream(stdout) << value << '\n';
                    if (!sidecars) QTextStream(stdout) << "--no-sidecars\n";
                }
                return rc;
            }
            return 0;
        }
    }
    if (parser.isSet(tray)) {
        QApplication::setQuitOnLastWindowClosed(false);
        Backend backend;
        SessionTray sessionTray(&backend, parser.value(appid), parser.value(log),
                                parser.value(gameName), parser.value(label),
                                parser.value(parentPid).toLongLong(), 0, true);
        sessionTray.show();
        return app.exec();
    }

    if (parser.isSet(install) && parser.isSet(shot)) {
        Backend backend;
        const QString path = parser.value(shot);
        QDialog* dialog = nullptr;
        if (parser.value(install) == "trucksbook") {
            dialog = new TrucksBookDialog(&backend, parser.value(appid));
        } else {
            dialog = new InstallerDialog(&backend, parser.value(appid));
        }
        dialog->show();
        QTimer::singleShot(800, [dialog, path] {
            dialog->grab().save(path);
            QApplication::quit();
        });
        return app.exec();
    }

    Backend backend;
    AppWindow window(&backend, parser.isSet(manage), parser.value(appid), parser.isSet(pick));
    window.show();
    if (parser.isSet(page)) window.openManagerPage(parser.value(page));
    if (parser.isSet(launch)) window.launchPath(parser.value(launch), false);
    if (parser.isSet(open)) window.openFile(parser.value(open));
    if (parser.isSet(install)) window.install(parser.value(install));

    if (parser.isSet(shot)) {
        const QString path = parser.value(shot);
        QTimer::singleShot(800, &window, [&window, path] {
            window.grab().save(path);
            QApplication::quit();
        });
    }
    return app.exec();
}
