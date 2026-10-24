#include "ipc.h"

#include <QDir>
#include <QElapsedTimer>
#include <QJsonDocument>
#include <QLocalSocket>
#include <QStandardPaths>
#include <unistd.h>

namespace ipc {

QString socketPath() {
    QString dir = qEnvironmentVariable("XDG_RUNTIME_DIR");
    if (dir.isEmpty()) dir = QDir::tempPath();
    return QDir(dir).filePath(QStringLiteral("steam-mp-launcher-gui-%1.sock")
                                  .arg(static_cast<qulonglong>(::getuid())));
}

QJsonObject request(const QString& mode, const QJsonObject& args, int timeoutMs) {
    QLocalSocket socket;
    socket.connectToServer(socketPath());
    if (!socket.waitForConnected(1500)) return {};
    const QJsonObject payload{{"mode", mode}, {"args", args}};
    socket.write(QJsonDocument(payload).toJson(QJsonDocument::Compact) + '\n');
    socket.flush();

    // One JSON object per line: keep reading until the newline arrives. A
    // single `readLine()` after `waitForReadyRead()` can parse half an object
    // and report "nobody answered" for a live window (Python reads in a loop).
    QElapsedTimer timer;
    timer.start();
    QByteArray buffer;
    for (;;) {
        buffer += socket.readAll();
        const int end = buffer.indexOf('\n');
        if (end >= 0) {
            const QJsonDocument reply =
                QJsonDocument::fromJson(buffer.left(end).trimmed());
            return reply.isObject() ? reply.object() : QJsonObject{};
        }
        int wait = 500;
        if (timeoutMs > 0) {
            const qint64 left = timeoutMs - timer.elapsed();
            if (left <= 0) return {};
            wait = static_cast<int>(qMin<qint64>(wait, left));
        }
        if (!socket.waitForReadyRead(wait)) {
            if (socket.state() != QLocalSocket::ConnectedState) return {};
            if (timeoutMs > 0 && timer.elapsed() >= timeoutMs) return {};
        }
    }
}

bool handoff(const QString& mode, const QJsonObject& args, QString* value, int* rc,
              int timeoutMs, bool* sidecars) {
    const QJsonObject reply = request(mode, args, timeoutMs);
    if (!reply.value("handled").toBool(false)) return false;
    if (mode == "pick") {
        if (value) *value = reply.value("value").toString();
        if (rc) *rc = reply.value("rc").toInt(1);
        if (sidecars) *sidecars = reply.value("sidecars").toBool(true);
    } else if (rc) {
        *rc = 0;
    }
    return true;
}

}  // namespace ipc
