#pragma once

#include <QHash>
#include <QJsonObject>
#include <QMainWindow>
#include <QPointer>

class Backend;
class PickerWidget;
class ManagerWidget;
class SessionTray;
class QStackedWidget;
class QLocalServer;
class QLocalSocket;

class AppWindow : public QMainWindow {
    Q_OBJECT
public:
    explicit AppWindow(Backend* backend, bool startManager = false,
                       const QString& appid = {}, bool helperMode = false,
                       QWidget* parent = nullptr);

    void showPicker();
    void showManager();
    void openManagerPage(const QString& page);
    void openFile(const QString& file);
    void runExtra(const QString& exe);
    void install(const QString& kind = {});
    void launchPath(const QString& exe, bool remember = false);

protected:
    void closeEvent(QCloseEvent* event) override;

private:
    void startIpc();
    void ipcNewConnection();
    void ipcRead(QLocalSocket* socket);
    void handleIpc(QLocalSocket* socket, const QByteArray& line);
    void replyIpc(QLocalSocket* socket, const QJsonObject& reply);
    void ipcDisconnected(QLocalSocket* socket);
    void launch(const QString& exe, bool remember);

    Backend* backend_ = nullptr;
    QStackedWidget* stack_ = nullptr;
    PickerWidget* picker_ = nullptr;
    ManagerWidget* manager_ = nullptr;
    QString appid_;
    bool helperMode_ = false;
    bool ownPick_ = false;
    SessionTray* tray_ = nullptr;
    QLocalServer* server_ = nullptr;
    QLocalSocket* pendingPick_ = nullptr;
    QHash<QLocalSocket*, QByteArray> ipcBuffers_;
};
