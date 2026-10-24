#pragma once

#include <QJsonObject>
#include <QString>

struct MpBackend;

/// Thin C++ wrapper around the Rust static library.  The boundary is JSON:
/// one command per call, one object back; no Qt types cross the ABI.
class Backend {
public:
    Backend();
    ~Backend();
    Backend(const Backend&) = delete;
    Backend& operator=(const Backend&) = delete;

    QJsonObject call(const QString& cmd, const QJsonObject& args = {}) const;
    QJsonObject picker(const QString& appid = {}) const;
    QJsonObject manager(const QString& appid = {}) const;
    QJsonObject launch(const QString& exe, bool remember, const QString& appid,
                       const QString& gameExe, bool sidecars = true) const;
    QJsonObject session(qint64 id) const;
    bool stopSession(qint64 id) const;

    static QString error(const QJsonObject& reply);
    static bool ok(const QJsonObject& reply) { return reply.value("ok").toBool(false); }

private:
    MpBackend* handle_ = nullptr;
};
