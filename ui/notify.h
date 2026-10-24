#pragma once

#include <QString>

namespace notify {

/// Desktop notification over the freedesktop D-Bus interface.
/// Fire-and-forget: never blocks the UI if no notification daemon runs.
void send(const QString& title, const QString& text);

}  // namespace notify
