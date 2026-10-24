#include "notify.h"

#include <QDBusConnection>
#include <QDBusMessage>
#include <QStringList>
#include <QVariantMap>

namespace notify {

void send(const QString& title, const QString& text) {
    QDBusMessage message = QDBusMessage::createMethodCall(
        QStringLiteral("org.freedesktop.Notifications"),
        QStringLiteral("/org/freedesktop/Notifications"),
        QStringLiteral("org.freedesktop.Notifications"), QStringLiteral("Notify"));
    message << QStringLiteral("Steam MP Launcher") << 0u
            << QStringLiteral("steam-mp-launcher") << title << text
            << QStringList{} << QVariantMap{} << 5000;
    // `send` is async: a missing notification daemon must not stall the app.
    QDBusConnection::sessionBus().send(message);
}

}  // namespace notify
