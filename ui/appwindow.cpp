#include "appwindow.h"
#include "i18n.h"
#include <QMessageBox>

#include "backend.h"
#include "installer_dialog.h"
#include "ipc.h"
#include "manager.h"
#include "openwith.h"
#include "picker.h"
#include "tray.h"
#include "trucksbook_dialog.h"

#include <QApplication>
#include <QFileInfo>
#include <QJsonDocument>
#include <QLocalServer>
#include <QLocalSocket>
#include <QCloseEvent>
#include <QTextStream>
#include <QStackedWidget>
#include <QTimer>

AppWindow::AppWindow(Backend* backend, bool startManager, const QString& appid,
                     bool helperMode, QWidget* parent)
    : QMainWindow(parent), backend_(backend), appid_(appid), helperMode_(helperMode) {
    stack_ = new QStackedWidget(this);
    setCentralWidget(stack_);
    picker_ = new PickerWidget(backend_, this);
    stack_->addWidget(picker_);
    connect(picker_, &PickerWidget::manageRequested, this, &AppWindow::showManager);
    connect(picker_, &PickerWidget::launchRequested, this, &AppWindow::launch);
    connect(picker_, &PickerWidget::installRequested, this, &AppWindow::install);
    connect(picker_, &PickerWidget::runOnceRequested, this, &AppWindow::runExtra);
    connect(picker_, &PickerWidget::cancelRequested, this, [this]() {
        if (pendingPick_) {
            QLocalSocket* client = pendingPick_;
            pendingPick_ = nullptr;
            replyIpc(client, {{"handled", true}, {"rc", 1}});
            if (manager_) {
                showManager();
            } else {
                close();
            }
            return;
        }
        if (helperMode_) {
            QApplication::exit(1);
        } else {
            close();
        }
    });
    if (!appid_.isEmpty()) picker_->setAppId(appid_);
    ownPick_ = !startManager;
    if (!helperMode_) startIpc();
    if (startManager) {
        showManager();
    } else {
        showPicker();
    }
    setWindowTitle(i18n("Steam MP Launcher"));
    setMinimumSize(520, 460);
}

void AppWindow::closeEvent(QCloseEvent* event) {
    if (pendingPick_) {
        replyIpc(pendingPick_, {{"handled", true}, {"rc", 1}});
        pendingPick_ = nullptr;
    }
    if (helperMode_) {
        QApplication::exit(1);
    }
    QMainWindow::closeEvent(event);
}

void AppWindow::startIpc() {
    if (server_) return;
    const QString path = ipc::socketPath();
    // Never steal the socket from a live window (manager/picker).
    QLocalSocket probe;
    probe.connectToServer(path);
    if (probe.waitForConnected(300)) {
        probe.abort();
        return;
    }
    QLocalServer::removeServer(path);
    server_ = new QLocalServer(this);
    if (!server_->listen(path)) {
        delete server_;
        server_ = nullptr;
        return;
    }
    connect(server_, &QLocalServer::newConnection, this, &AppWindow::ipcNewConnection);
}

void AppWindow::ipcNewConnection() {
    while (server_ && server_->hasPendingConnections()) {
        QLocalSocket* socket = server_->nextPendingConnection();
        ipcBuffers_.insert(socket, {});
        connect(socket, &QLocalSocket::readyRead, this, [this, socket] { ipcRead(socket); });
        connect(socket, &QLocalSocket::disconnected, this, [this, socket] { ipcDisconnected(socket); });
    }
}

void AppWindow::ipcRead(QLocalSocket* socket) {
    QByteArray& buffer = ipcBuffers_[socket];
    buffer += socket->readAll();
    int end = -1;
    while ((end = buffer.indexOf('\n')) >= 0) {
        const QByteArray line = buffer.left(end);
        buffer.remove(0, end + 1);
        handleIpc(socket, line);
    }
}

void AppWindow::handleIpc(QLocalSocket* socket, const QByteArray& line) {
    const QJsonDocument doc = QJsonDocument::fromJson(line);
    if (!doc.isObject()) {
        replyIpc(socket, {{"handled", false}});
        return;
    }
    const QJsonObject payload = doc.object();
    const QString mode = payload.value("mode").toString();
    const QJsonObject args = payload.value("args").toObject();
    if (mode == "manage") {
        showManager();
        replyIpc(socket, {{"handled", true}, {"done", true}});
        return;
    }
    if (mode != "pick") {
        replyIpc(socket, {{"handled", false}});
        return;
    }
    if (ownPick_ || pendingPick_) {
        replyIpc(socket, {{"handled", false}});
        return;
    }
    const QString appid = args.value("appid").toString();
    if (!appid.isEmpty()) picker_->setAppId(appid);
    pendingPick_ = socket;
    showPicker();
}

void AppWindow::replyIpc(QLocalSocket* socket, const QJsonObject& reply) {
    if (!socket) return;
    ipcBuffers_.remove(socket);
    if (pendingPick_ == socket) pendingPick_ = nullptr;
    socket->write(QJsonDocument(reply).toJson(QJsonDocument::Compact) + '\n');
    socket->flush();
    socket->disconnectFromServer();
    socket->deleteLater();
}

void AppWindow::ipcDisconnected(QLocalSocket* socket) {
    ipcBuffers_.remove(socket);
    if (pendingPick_ == socket) {
        pendingPick_ = nullptr;
        if (manager_) {
            showManager();
        }
    }
    socket->deleteLater();
}

void AppWindow::showPicker() {
    picker_->reload();
    stack_->setCurrentWidget(picker_);
    resize(600, 560);
    setWindowTitle(i18n("%1 · Steam MP Launcher", picker_->appId()));
    show();
    raise();
}

void AppWindow::showManager() {
    if (!manager_) {
        manager_ = new ManagerWidget(backend_, this);
        stack_->addWidget(manager_);
        connect(manager_, &ManagerWidget::backToPicker, this, &AppWindow::showPicker);
        connect(manager_, &ManagerWidget::installRequested, this, &AppWindow::install);
    }
    manager_->setAppId(picker_->appId());
    stack_->setCurrentWidget(manager_);
    resize(960, 660);
    setWindowTitle(i18n("Manager · Steam MP Launcher"));
    show();
    raise();
}

void AppWindow::openManagerPage(const QString& page) {
    showManager();
    manager_->switchTo(page);
}

void AppWindow::openFile(const QString& file) {
    OpenWithDialog dialog(backend_, file, this);
    if (dialog.exec() != QDialog::Accepted) return;
    if (dialog.runOnce()) {
        picker_->setAppId(dialog.appId());
        runExtra(file);
    } else {
        picker_->reload();
    }
}

/// Same path as the tray's "Run another .exe…": Proton's `runinprefix` in the
/// game's prefix, detached, no session/tray window.
void AppWindow::runExtra(const QString& exe) {
    const QJsonObject state = backend_->picker(picker_->appId());
    const QJsonObject reply = backend_->call("run_extra", {
        {"appid", picker_->appId()},
        {"compatdata", state.value("game").toObject().value("compatdata")},
        {"exe", exe},
    });
    if (!Backend::ok(reply)) {
        QMessageBox::warning(this, i18n("Could not start the program"), Backend::error(reply));
    }
}

void AppWindow::launchPath(const QString& exe, bool remember) {
    launch(exe, remember);
}

void AppWindow::launch(const QString& exe, bool remember) {
    if (pendingPick_) {
        if (remember && !exe.isEmpty() && exe != QStringLiteral("__GAME__")) {
            backend_->call("save_choice", {{"appid", picker_->appId()}, {"path", exe}});
        }
        QLocalSocket* client = pendingPick_;
        pendingPick_ = nullptr;
        replyIpc(client, {{"handled", true},
                          {"rc", 0},
                          {"value", exe},
                          {"sidecars", picker_->startSidecars()}});
        if (manager_) {
            showManager();
        } else {
            close();
        }
        return;
    }
    if (helperMode_) {
        if (remember && !exe.isEmpty() && exe != QStringLiteral("__GAME__")) {
            backend_->call("save_choice", {{"appid", picker_->appId()}, {"path", exe}});
        }
        QTextStream(stdout) << (exe.isEmpty() ? QStringLiteral("__GAME__") : exe) << '\n';
        if (!picker_->startSidecars()) QTextStream(stdout) << "--no-sidecars\n";
        QApplication::exit(0);
        return;
    }
    const QJsonObject reply =
        backend_->launch(exe, remember, picker_->appId(), {}, picker_->startSidecars());
    if (!Backend::ok(reply)) {
        QMessageBox::critical(this, i18n("Launch failed"), Backend::error(reply));
        return;
    }
    const QJsonObject session = reply.value("session").toObject();
    const qint64 id = static_cast<qint64>(reply.value("id").toDouble());
    const QString label = (exe.isEmpty() || exe == QStringLiteral("__GAME__"))
                              ? i18n("The game")
                              : QFileInfo(exe).completeBaseName();
    hide();
    tray_ = new SessionTray(backend_, picker_->appId(), session.value("log").toString(),
                            session.value("game").toString(), label, 0, id, false, this);
    connect(tray_, &SessionTray::finished, this, [this] {
        if (tray_) {
            tray_->deleteLater();
            tray_ = nullptr;
        }
        show();
        showPicker();
    });
    tray_->show();
}

void AppWindow::install(const QString& kind) {
    // The installers also live in Settings: then the manager's game decides.
    const bool fromManager = manager_ && stack_->currentWidget() == manager_;
    const QString appid = fromManager ? manager_->appId()
                                      : (picker_ ? picker_->appId() : QString());
    QString added;
    if (kind == "trucksbook") {
        TrucksBookDialog dialog(backend_, appid, this);
        dialog.exec();
        added = dialog.addedPath();
    } else {
        InstallerDialog dialog(backend_, appid, this);
        dialog.exec();
        added = dialog.addedPath();
    }
    if (!added.isEmpty() && picker_) picker_->reload(added);
    if (!added.isEmpty() && fromManager && manager_) manager_->reload();
}
