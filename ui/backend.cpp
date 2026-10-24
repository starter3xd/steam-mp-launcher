#include "backend.h"

#include "mp_backend.h"

#include <QByteArray>
#include <QDebug>
#include <QElapsedTimer>
#include <QJsonDocument>
#include <QJsonParseError>

Backend::Backend() : handle_(mp_backend_new()) {}

Backend::~Backend() {
    if (handle_) {
        mp_backend_free(handle_);
    }
}

QJsonObject Backend::call(const QString& cmd, const QJsonObject& args) const {
    if (!handle_) {
        return {{"ok", false}, {"error", "backend is not initialised"}};
    }
    QJsonObject request = args;
    request.insert("cmd", cmd);
    const QByteArray payload = QJsonDocument(request).toJson(QJsonDocument::Compact);
    QElapsedTimer timer;
    timer.start();
    char* raw = mp_backend_call(handle_, payload.constData());
    const QByteArray reply(raw ? raw : "");
    mp_backend_free_string(raw);
    // A slow command runs on the UI thread; make it visible in the journal
    // instead of looking like a frozen window.
    if (timer.elapsed() > 500) {
        qWarning("backend %s took %lld ms", qPrintable(cmd),
                 static_cast<long long>(timer.elapsed()));
    }

    QJsonParseError error{};
    const QJsonDocument doc = QJsonDocument::fromJson(reply, &error);
    if (error.error != QJsonParseError::NoError || !doc.isObject()) {
        return {{"ok", false}, {"error", "invalid backend reply: " + error.errorString()}};
    }
    return doc.object();
}

QJsonObject Backend::picker(const QString& appid) const {
    QJsonObject args;
    if (!appid.isEmpty()) args.insert("appid", appid);
    return call("picker", args);
}

QJsonObject Backend::manager(const QString& appid) const {
    QJsonObject args;
    if (!appid.isEmpty()) args.insert("appid", appid);
    return call("manager", args);
}

QJsonObject Backend::launch(const QString& exe, bool remember, const QString& appid,
                            const QString& gameExe, bool sidecars) const {
    QJsonObject args{{"exe", exe}, {"remember", remember}, {"sidecars", sidecars}};
    if (!appid.isEmpty()) args.insert("appid", appid);
    if (!gameExe.isEmpty()) args.insert("game_exe", gameExe);
    return call("launch", args);
}

QJsonObject Backend::session(qint64 id) const {
    return call("session", {{"id", id}});
}

bool Backend::stopSession(qint64 id) const {
    return ok(call("stop_session", {{"id", id}}));
}

QString Backend::error(const QJsonObject& reply) {
    return reply.value("error").toString("unknown backend error");
}
