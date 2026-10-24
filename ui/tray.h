#pragma once

#include <QObject>
#include <QString>
#include <QVector>

class Backend;
class QMenu;
class KStatusNotifierItem;
class QTimer;

/// The only per-session UI: a tray icon.  It can start additional `.exe`
/// files through Proton and stop the session; there is deliberately no
/// session window.
class SessionTray : public QObject {
    Q_OBJECT
public:
    SessionTray(Backend* backend, const QString& appid, const QString& logPath,
                const QString& game, const QString& label, qint64 parentPid,
                qint64 sessionId, bool quitOnFinish, QObject* parent = nullptr);
    ~SessionTray() override;

    void show();

signals:
    void finished();

private:
    void buildMenu();
    void poll();
    void runExtra(const QString& path);
    void runAnother();
    void stop();
    void finish();

    Backend* backend_ = nullptr;
    QString appid_;
    QString compatdata_;
    QString logPath_;
    QString game_;
    QString label_;
    qint64 parentPid_ = 0;
    qint64 sessionId_ = 0;
    bool quitOnFinish_ = false;
    bool done_ = false;
    qint64 offset_ = 0;

    struct Extra {
        QString label;
        QString path;
        QString icon;
    };

    KStatusNotifierItem* tray_ = nullptr;
    QMenu* menu_ = nullptr;
    QTimer* timer_ = nullptr;
    QVector<Extra> extras_;
};
