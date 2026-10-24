#pragma once

#include <QCoreApplication>
#include <QString>
#include <QVariantList>

/// The one bit of Ki18n we used: a translation lookup plus `%1`…`%n`
/// substitution, so the UI stays dependency-free (Qt only).  There are no
/// translation catalogs yet; Qt returns the source string.
inline QString i18n(const char* text) {
    return QCoreApplication::translate("steam-mp-launcher", text);
}

template <typename... Args>
QString i18n(const char* text, const Args&... args) {
    QString out = i18n(text);
    const QVariantList values{args...};
    for (int index = 0; index < values.size(); ++index) {
        out.replace(QStringLiteral("%") + QString::number(index + 1),
                    values.at(index).toString());
    }
    return out;
}
