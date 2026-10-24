#include "tray.h"
#include "i18n.h"

#include "backend.h"
#include "theme.h"
#include "notify.h"

#include <QApplication>
#include <QCursor>
#include <QDir>
#include <QFile>
#include <QFileDialog>
#include <QFileInfo>
#include <QIcon>
#include <QJsonArray>
#include <QJsonObject>
#include <QMenu>
#include <QSystemTrayIcon>
#include <KStatusNotifierItem>
#include <QTimer>
#include <csignal>
#include <unistd.h>

SessionTray::SessionTray(Backend* backend, const QString& appid, const QString& logPath,
                         const QString& game, const QString& label, qint64 parentPid,
                         qint64 sessionId, bool quitOnFinish, QObject* parent)
    : QObject(parent),
      backend_(backend),
      appid_(appid),
      logPath_(logPath),
      game_(game),
      label_(label),
      parentPid_(parentPid),
      sessionId_(sessionId),
      quitOnFinish_(quitOnFinish) {
    const QJsonObject state = backend_->call("picker", appid_.isEmpty()
                                                          ? QJsonObject{}
                                                          : QJsonObject{{"appid", appid_}});
    const QJsonObject gameState = state.value("game").toObject();
    if (game_.isEmpty()) game_ = gameState.value("name").toString(i18n("Game"));
    compatdata_ = gameState.value("compatdata").toString();
    if (label_.isEmpty()) label_ = game_;

    const QJsonArray extras = state.value("extras").toArray();
    for (const QJsonValue& value : extras) {
        const QJsonObject entry = value.toObject();
        const QString path = entry.value("path").toString();
        if (path.isEmpty()) continue;
        extras_.append({entry.value("label").toString(QFileInfo(path).completeBaseName()),
                        path, entry.value("icon").toString()});
    }

    buildMenu();

    tray_ = new KStatusNotifierItem(QStringLiteral("steam-mp-launcher-session"), this);
    tray_->setCategory(KStatusNotifierItem::ApplicationStatus);
    // Hand Plasma the pixels, not just a name: when the host cannot resolve
    // the icon name it falls back to the icon of the application that
    // started us (Steam), which is exactly the wrong logo.
    tray_->setIconByPixmap(theme::appIcon());
    tray_->setToolTip(QStringLiteral("steam-mp-launcher"), i18n("%1 · %2", label_, game_),
                      i18n("Steam MP Launcher"));
    tray_->setContextMenu(menu_);
    connect(tray_, &KStatusNotifierItem::activateRequested, this,
            [this](bool, const QPoint&) { menu_->popup(QCursor::pos()); });
    connect(tray_, &KStatusNotifierItem::secondaryActivateRequested, this,
            [this](const QPoint&) { menu_->popup(QCursor::pos()); });

    timer_ = new QTimer(this);
    timer_->setInterval(500);
    connect(timer_, &QTimer::timeout, this, &SessionTray::poll);
    timer_->start();
}

SessionTray::~SessionTray() {
    if (timer_) timer_->stop();
    if (tray_) {
        // The SNI item holds a raw pointer to the menu and the tray object is
        // destroyed after this body: detach it before the menu goes away.
        tray_->setContextMenu(nullptr);
        tray_->setStatus(KStatusNotifierItem::Passive);
    }
    delete menu_;
    menu_ = nullptr;
}

void SessionTray::buildMenu() {
    menu_ = new QMenu();
    auto* head = menu_->addAction(i18n("%1 · %2", label_, game_));
    head->setEnabled(false);

    if (!extras_.isEmpty()) {
        menu_->addSeparator();
        for (const auto& extra : extras_) {
            auto* action = menu_->addAction(extra.label);
            if (!extra.icon.isEmpty()) action->setIcon(QIcon(extra.icon));
            action->setToolTip(extra.path);
            connect(action, &QAction::triggered, this,
                    [this, path = extra.path](bool) { runExtra(path); });
        }
    }

    menu_->addSeparator();
    auto* another = menu_->addAction(i18n("Run another .exe…"));
    connect(another, &QAction::triggered, this, [this](bool) { runAnother(); });

    menu_->addSeparator();
    auto* stopAction = menu_->addAction(i18n("Stop game"));
    connect(stopAction, &QAction::triggered, this, [this](bool) { stop(); });
}

void SessionTray::show() {
    if (!tray_) return;
    if (!QSystemTrayIcon::isSystemTrayAvailable()) {
        // No tray (rare on KDE): do not leave a helper process behind.
        finish();
        return;
    }
    tray_->setStatus(KStatusNotifierItem::Active);
}

void SessionTray::poll() {
    if (done_) return;
    if (parentPid_ > 0 && ::kill(static_cast<pid_t>(parentPid_), 0) != 0) {
        finish();
        return;
    }
    // The session state is the source of truth; the log is only the fallback
    // for the runner's own tray (a different process, sessionId_ == 0).
    if (sessionId_ > 0) {
        const QJsonObject state = backend_->call("session", {{"id", sessionId_}});
        if (Backend::ok(state) && state.value("done").toBool(false)) {
            finish();
            return;
        }
    }
    QFile file(logPath_);
    if (!file.open(QIODevice::ReadOnly)) return;
    if (file.size() < offset_) offset_ = 0;
    if (!file.seek(offset_)) return;
    const QByteArray chunk = file.readAll();
    offset_ = file.pos();
    if (chunk.contains("done :") || chunk.contains("done (rc=")) finish();
}

void SessionTray::runExtra(const QString& path) {
    const QJsonObject reply = backend_->call("run_extra", {
        {"appid", appid_},
        {"compatdata", compatdata_},
        {"exe", path},
    });
    if (!Backend::ok(reply)) {
        notify::send(i18n("Could not start the program"), Backend::error(reply));
    }
}

void SessionTray::runAnother() {
    QString start = QDir::homePath();
    if (!extras_.isEmpty()) start = QFileInfo(extras_.first().path).absolutePath();
    const QString path = QFileDialog::getOpenFileName(
        nullptr, i18n("Run an additional program"), start, i18n("Windows programs (*.exe)"));
    if (!path.isEmpty()) runExtra(path);
}

void SessionTray::stop() {
    if (sessionId_ > 0) {
        backend_->stopSession(sessionId_);
    } else if (parentPid_ > 0) {
        ::kill(static_cast<pid_t>(parentPid_), SIGTERM);
    }
    if (tray_) {
        tray_->setToolTip(QStringLiteral("applications-games"),
                          i18n("%1 · stopping…", label_), i18n("Steam MP Launcher"));
    }
}

void SessionTray::finish() {
    if (done_) return;
    done_ = true;
    if (timer_) timer_->stop();
    if (tray_) {
        tray_->setToolTip(QStringLiteral("applications-games"),
                          i18n("%1 · finished", label_), i18n("Steam MP Launcher"));
        tray_->setStatus(KStatusNotifierItem::Passive);
    }
    emit finished();
    if (quitOnFinish_) QTimer::singleShot(1200, qApp, &QCoreApplication::quit);
}
