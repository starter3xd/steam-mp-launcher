#pragma once

#include <QJsonObject>
#include <QString>

namespace ipc {

QString socketPath();

/// Send one request to an already-open window.  Returns an empty object when
/// no window answers (the caller should start its own UI).
QJsonObject request(const QString& mode, const QJsonObject& args, int timeoutMs = 700);

bool handoff(const QString& mode, const QJsonObject& args, QString* value, int* rc,
              int timeoutMs = 700, bool* sidecars = nullptr);

}  // namespace ipc
