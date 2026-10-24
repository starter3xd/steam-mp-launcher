#include "theme.h"

#include <QApplication>
#include <QCoreApplication>
#include <QDBusConnection>
#include <QDBusMessage>
#include <QDBusVariant>
#include <QFileInfo>
#include <QFontMetrics>
#include <QGuiApplication>
#include <QIcon>
#include <QLinearGradient>
#include <QPainter>
#include <QPixmap>
#include <QProcess>
#include <QStandardPaths>
#include <QStyle>
#include <QStyleFactory>
#include <QStyleHints>
#include <QStringList>
#include <QVariant>

namespace theme {

namespace {

enum class Scheme { Unknown, Light, Dark };

Scheme schemeOf(int value) {
    if (value == 1) return Scheme::Dark;
    if (value == 2) return Scheme::Light;
    return Scheme::Unknown;
}

const char* schemeName(Scheme scheme) {
    switch (scheme) {
    case Scheme::Dark: return "dark";
    case Scheme::Light: return "light";
    default: return "unknown";
    }
}

// A short-lived call to an external tool; empty when it is missing or slow.
QString runTool(const QString& program, const QStringList& arguments) {
    if (QStandardPaths::findExecutable(program).isEmpty()) return {};
    QProcess process;
    process.start(program, arguments);
    if (!process.waitForFinished(700)) {
        process.kill();
        process.waitForFinished(200);
        return {};
    }
    if (process.exitStatus() != QProcess::NormalExit || process.exitCode() != 0) return {};
    return QString::fromUtf8(process.readAllStandardOutput()).trimmed();
}

QString gsetting(const QString& key) {
    QString value = runTool(QStringLiteral("gsettings"),
                            {QStringLiteral("get"), QStringLiteral("org.gnome.desktop.interface"), key});
    if (value.size() >= 2 && value.startsWith(QLatin1Char('\'')) && value.endsWith(QLatin1Char('\'')))
        value = value.mid(1, value.size() - 2);
    return value;
}

// org.freedesktop.appearance color-scheme: 0 no preference, 1 dark, 2 light.
// GNOME (and Flatpak, KDE) answer this through xdg-desktop-portal; Qt itself
// only asks when a GTK/KDE platform theme plugin is loaded.
Scheme portalScheme() {
    QDBusConnection bus = QDBusConnection::sessionBus();
    if (!bus.isConnected()) return Scheme::Unknown;
    QDBusMessage call = QDBusMessage::createMethodCall(
        QStringLiteral("org.freedesktop.portal.Desktop"),
        QStringLiteral("/org/freedesktop/portal/desktop"),
        QStringLiteral("org.freedesktop.portal.Settings"), QStringLiteral("Read"));
    call << QStringLiteral("org.freedesktop.appearance") << QStringLiteral("color-scheme");
    const QDBusMessage reply = bus.call(call, QDBus::Block, 500);
    if (reply.type() != QDBusMessage::ReplyMessage || reply.arguments().isEmpty())
        return Scheme::Unknown;
    QVariant value = reply.arguments().first();
    // Settings.Read answers with a variant that may itself hold a variant.
    for (int depth = 0; depth < 4 && value.canConvert<QDBusVariant>(); ++depth) {
        const QVariant inner = value.value<QDBusVariant>().variant();
        if (inner == value) break;
        value = inner;
    }
    bool ok = false;
    const int number = value.toInt(&ok);
    return ok ? schemeOf(number) : Scheme::Unknown;
}

Scheme qtScheme() {
#if QT_VERSION >= QT_VERSION_CHECK(6, 5, 0)
    switch (QGuiApplication::styleHints()->colorScheme()) {
    case Qt::ColorScheme::Dark: return Scheme::Dark;
    case Qt::ColorScheme::Light: return Scheme::Light;
    default: break;
    }
#endif
    return Scheme::Unknown;
}

Scheme gtkScheme() {
    // GNOME 42+: "default" (follow the theme) or "prefer-dark"/"prefer-light".
    const QString preference = gsetting(QStringLiteral("color-scheme"));
    if (preference == QLatin1String("prefer-dark")) return Scheme::Dark;
    if (preference == QLatin1String("prefer-light")) return Scheme::Light;
    // GTK3-era signal: an explicit dark theme.
    const QString theme = gsetting(QStringLiteral("gtk-theme"));
    if (theme.contains(QLatin1String("dark"), Qt::CaseInsensitive)) return Scheme::Dark;
    if (qEnvironmentVariable("GTK_THEME").contains(QLatin1String("dark"), Qt::CaseInsensitive))
        return Scheme::Dark;
    return Scheme::Unknown;
}

struct Detection {
    QString forced;  // STEAM_MP_THEME
    Scheme portal = Scheme::Unknown;
    Scheme qt = Scheme::Unknown;
    QString colorScheme;
    QString gtkTheme;
    QString iconTheme;
    Scheme decided = Scheme::Light;
    QString reason = QStringLiteral("default");
};

// The portal is the authority on the desktop preference (that is what
// libadwaita and Flatpak apps follow); the Qt hint covers KDE, where the
// platform theme already built a matching palette, so nothing gets changed
// there; gsettings covers old distros without a portal.
Detection detect(bool verbose) {
    Detection d;
    d.forced = qEnvironmentVariable("STEAM_MP_THEME").trimmed().toLower();
    d.portal = portalScheme();
    d.qt = qtScheme();

    if (d.forced == QLatin1String("dark") || d.forced == QLatin1String("light")) {
        d.decided = d.forced == QLatin1String("dark") ? Scheme::Dark : Scheme::Light;
        d.reason = QStringLiteral("STEAM_MP_THEME");
    } else if (d.portal != Scheme::Unknown) {
        d.decided = d.portal;
        d.reason = QStringLiteral("portal");
    } else if (d.qt != Scheme::Unknown) {
        d.decided = d.qt;
        d.reason = QStringLiteral("qt platform theme");
    } else {
        const Scheme gtk = gtkScheme();
        if (gtk != Scheme::Unknown) {
            d.decided = gtk;
            d.reason = QStringLiteral("gsettings");
        }
    }

    if (verbose) {
        if (d.colorScheme.isEmpty()) d.colorScheme = gsetting(QStringLiteral("color-scheme"));
        if (d.gtkTheme.isEmpty()) d.gtkTheme = gsetting(QStringLiteral("gtk-theme"));
        if (d.iconTheme.isEmpty()) d.iconTheme = gsetting(QStringLiteral("icon-theme"));
    }
    return d;
}

bool paletteIsDark() {
    return QApplication::palette().color(QPalette::Window).lightness() < 128;
}

// Breeze Dark values, so a GNOME session and a Plasma session look alike.
QPalette darkPalette() {
    const QColor window(0x31, 0x36, 0x3b);
    const QColor base(0x23, 0x26, 0x29);
    const QColor text(0xef, 0xf0, 0xf1);
    const QColor muted(0x7f, 0x8c, 0x8d);
    const QColor highlight(0x3d, 0xae, 0xe9);
    QPalette palette;
    const auto set = [&palette](QPalette::ColorRole role, const QColor& color) {
        for (auto group : {QPalette::Active, QPalette::Inactive, QPalette::Disabled})
            palette.setColor(group, role, color);
    };
    set(QPalette::Window, window);
    set(QPalette::WindowText, text);
    set(QPalette::Base, base);
    set(QPalette::AlternateBase, window.lighter(108));
    set(QPalette::ToolTipBase, window);
    set(QPalette::ToolTipText, text);
    set(QPalette::Text, text);
    set(QPalette::Button, window);
    set(QPalette::ButtonText, text);
    set(QPalette::BrightText, Qt::white);
    set(QPalette::Link, highlight);
    set(QPalette::Highlight, highlight);
    set(QPalette::HighlightedText, Qt::white);
    set(QPalette::PlaceholderText, muted);
    set(QPalette::Light, window.lighter(160));
    set(QPalette::Midlight, window.lighter(130));
    set(QPalette::Mid, window.darker(130));
    set(QPalette::Dark, window.darker(160));
    set(QPalette::Shadow, Qt::black);
#if QT_VERSION >= QT_VERSION_CHECK(6, 6, 0)
    set(QPalette::Accent, highlight);
#endif
    for (auto role : {QPalette::WindowText, QPalette::Text, QPalette::ButtonText,
                      QPalette::HighlightedText, QPalette::PlaceholderText})
        palette.setColor(QPalette::Disabled, role, muted);
    for (auto role : {QPalette::Window, QPalette::Base, QPalette::Button})
        palette.setColor(QPalette::Disabled, role, window);
    return palette;
}

void setQtColorScheme(Scheme scheme) {
#if QT_VERSION >= QT_VERSION_CHECK(6, 8, 0)
    QGuiApplication::styleHints()->setColorScheme(scheme == Scheme::Dark ? Qt::ColorScheme::Dark
                                                                        : Qt::ColorScheme::Light);
#else
    Q_UNUSED(scheme);
#endif
}

bool iconThemeExists(const QString& name) {
    if (name.isEmpty()) return false;
    const QString entry = QStringLiteral("icons/%1/index.theme").arg(name);
    if (!QStandardPaths::locate(QStandardPaths::GenericDataLocation, entry).isEmpty()) return true;
    for (const QString& root : QIcon::themeSearchPaths()) {
        if (!root.isEmpty() &&
            QFileInfo::exists(root + QLatin1Char('/') + name + QLatin1String("/index.theme")))
            return true;
    }
    return false;
}

// Without a GTK platform plugin Qt only looks at `hicolor`, which on GNOME has
// almost nothing, so the toolbar would be a row of generic file icons.  Use the
// desktop's own theme (never override one a platform theme already picked).
void ensureIconSearchPaths() {
    QStringList paths = QIcon::themeSearchPaths();
    for (const QString& dir : QStandardPaths::standardLocations(QStandardPaths::GenericDataLocation)) {
        const QString icons = dir + QLatin1String("/icons");
        if (!paths.contains(icons)) paths << icons;
    }
    if (!paths.contains(QStringLiteral("/usr/share/pixmaps")))
        paths << QStringLiteral("/usr/share/pixmaps");
    if (paths != QIcon::themeSearchPaths()) QIcon::setThemeSearchPaths(paths);
}

void applyIconTheme(const Detection& d) {
    ensureIconSearchPaths();
    const QString current = QIcon::themeName();
    if (!current.isEmpty() && current != QLatin1String("hicolor")) return;
    QStringList candidates;
    const QString preferred = d.iconTheme.isEmpty()
                                  ? gsetting(QStringLiteral("icon-theme"))
                                  : d.iconTheme;
    if (!preferred.isEmpty()) candidates << preferred;
    candidates << QStringLiteral("Adwaita") << QStringLiteral("Yaru")
               << QStringLiteral("breeze") << QStringLiteral("Papirus")
               << QStringLiteral("hicolor");
    for (const QString& candidate : candidates) {
        if (!iconThemeExists(candidate)) continue;
        QIcon::setThemeName(candidate);
        return;
    }
}

}  // namespace

QString& lastAction() {
    static QString action = QStringLiteral("nothing to do");
    return action;
}

void applySystemTheme() {
    const Detection d = detect(false);
    if (d.decided == Scheme::Dark) {
        // A dark platform palette (KDE, a GTK platform theme) is left alone.
        if (paletteIsDark()) {
            lastAction() = QStringLiteral("kept the platform palette");
        } else {
            QApplication::setStyle(QStyleFactory::create(QStringLiteral("Fusion")));
            QApplication::setPalette(darkPalette());
            setQtColorScheme(Scheme::Dark);
            lastAction() = QStringLiteral("applied the dark palette");
        }
    } else if (d.forced == QLatin1String("light") && paletteIsDark()) {
        QApplication::setStyle(QStyleFactory::create(QStringLiteral("Fusion")));
        QApplication::setPalette(QPalette());
        setQtColorScheme(Scheme::Light);
        lastAction() = QStringLiteral("applied the light palette");
    }
    applyIconTheme(d);
}

QString systemThemeReport() {
    const Detection d = detect(true);
    const QColor window = QApplication::palette().color(QPalette::Window);
    QStringList lines;
    lines << QStringLiteral("STEAM_MP_THEME: %1")
                 .arg(d.forced.isEmpty() ? QStringLiteral("(unset)") : d.forced);
    lines << QStringLiteral("portal color-scheme: %1").arg(QLatin1String(schemeName(d.portal)));
    lines << QStringLiteral("qt color scheme: %1").arg(QLatin1String(schemeName(d.qt)));
    lines << QStringLiteral("gsettings color-scheme: %1")
                 .arg(d.colorScheme.isEmpty() ? QStringLiteral("(none)") : d.colorScheme);
    lines << QStringLiteral("gsettings gtk-theme: %1")
                 .arg(d.gtkTheme.isEmpty() ? QStringLiteral("(none)") : d.gtkTheme);
    lines << QStringLiteral("gsettings icon-theme: %1")
                 .arg(d.iconTheme.isEmpty() ? QStringLiteral("(none)") : d.iconTheme);
    lines << QStringLiteral("icon theme in use: %1").arg(QIcon::themeName());
    lines << QStringLiteral("style: %1").arg(QApplication::style()->objectName());
    lines << QStringLiteral("window colour: %1 (lightness %2)")
                 .arg(window.name())
                 .arg(window.lightness());
    lines << QStringLiteral("decision: %1 (%2)")
                 .arg(QLatin1String(schemeName(d.decided)), d.reason);
    lines << QStringLiteral("action: %1").arg(lastAction());
    return lines.join(QLatin1Char('\n')) + QLatin1Char('\n');
}

QColor dim() { return QApplication::palette().color(QPalette::PlaceholderText); }
namespace {
bool darkUi() {
    return QApplication::palette().color(QPalette::Window).lightness() < 128;
}
}  // namespace

// Semantic colours without KColorScheme: readable on both Breeze Dark and
// light themes.
QColor negative() { return darkUi() ? QColor(0xff, 0x7b, 0x72) : QColor(0xcf, 0x22, 0x2e); }
QColor positive() { return darkUi() ? QColor(0x7e, 0xd1, 0x88) : QColor(0x1a, 0x7f, 0x37); }
QColor neutral() { return darkUi() ? QColor(0xf0, 0xa3, 0x5e) : QColor(0xbf, 0x6a, 0x02); }

void recolor(QWidget* widget, const QColor& color, QPalette::ColorRole role) {
    QPalette palette = widget->palette();
    for (auto group : {QPalette::Active, QPalette::Inactive}) {
        palette.setColor(group, role, color);
    }
    widget->setPalette(palette);
}

QLabel* dim(QLabel* label) {
    recolor(label, label->palette().color(QPalette::PlaceholderText));
    return label;
}

QFont boldFont(const QFont& base, qreal delta) {
    QFont font = base;
    if (delta != 0.0) font.setPointSizeF(font.pointSizeF() + delta);
    font.setBold(true);
    return font;
}

namespace {
// Name differences between icon themes: Adwaita has no `view-list-details`,
// Breeze has both.
QString aliasFor(const QString& name) {
    if (name == QLatin1String("view-list-details")) return QStringLiteral("view-list");
    return {};
}

// GNOME/Adwaita ships many icons only as `*-symbolic`: monochrome glyphs that
// Qt does not tint by itself, so paint them with the text colour (otherwise
// they vanish on light or dark themes).
QIcon symbolicIcon(const QString& name, int size) {
    const QIcon symbolic = QIcon::fromTheme(name + QStringLiteral("-symbolic"));
    if (symbolic.isNull()) return {};
    QImage image = symbolic.pixmap(size, size).toImage().convertToFormat(QImage::Format_ARGB32);
    QPainter painter(&image);
    painter.setCompositionMode(QPainter::CompositionMode_SourceIn);
    painter.fillRect(image.rect(), QApplication::palette().color(QPalette::WindowText));
    painter.end();
    return QIcon(QPixmap::fromImage(image));
}
}  // namespace

QIcon icon(const QString& name, int size) {
    const QString alias = aliasFor(name);
    QIcon result = QIcon::fromTheme(name);
    if (result.isNull() && !alias.isEmpty()) result = QIcon::fromTheme(alias);
    if (result.isNull()) result = symbolicIcon(name, size);
    if (result.isNull() && !alias.isEmpty()) result = symbolicIcon(alias, size);
    if (result.isNull()) {
        result = QApplication::style()->standardIcon(QStyle::SP_FileIcon);
    }
    const QPixmap pixmap = result.pixmap(size, size);
    QIcon scaled;
    scaled.addPixmap(pixmap);
    return scaled;
}

QIcon appIcon() {
    QIcon icon = QIcon::fromTheme(QStringLiteral("steam-mp-launcher"));
    if (icon.isNull()) {
        const QString dir = QCoreApplication::applicationDirPath();
        for (const QString& path : {
                 dir + QStringLiteral("/steam-mp-launcher.svg"),
                 dir + QStringLiteral("/../dist/steam-mp-launcher.svg"),
             }) {
            if (!QFileInfo::exists(path)) continue;
            icon = QIcon(path);
            if (!icon.isNull()) break;
        }
    }
    if (icon.isNull()) icon = QIcon::fromTheme(QStringLiteral("applications-games"));
    if (icon.isNull()) icon = QApplication::style()->standardIcon(QStyle::SP_ComputerIcon);
    return icon;
}

QPixmap fileIcon(const QString& path, int size) {
    if (!path.isEmpty()) {
        const QPixmap pixmap(path);
        if (!pixmap.isNull()) {
            return pixmap.scaled(size, size, Qt::KeepAspectRatio, Qt::SmoothTransformation);
        }
    }
    return icon(QStringLiteral("application-x-executable"), size).pixmap(size, size);
}

QString elideMiddle(const QFontMetrics& metrics, const QString& text, int width) {
    return metrics.elidedText(text, Qt::ElideMiddle, width);
}

void paintBanner(QWidget* widget, QPainter& painter, const QString& path) {
    const QPixmap banner(path);
    if (banner.isNull()) {
        painter.fillRect(widget->rect(), widget->palette().color(QPalette::Window));
        return;
    }
    const QRect target = widget->rect();
    const QPixmap scaled = banner.scaled(target.size(), Qt::KeepAspectRatioByExpanding,
                                        Qt::SmoothTransformation);
    const QRect source((scaled.width() - target.width()) / 2,
                       (scaled.height() - target.height()) / 2,
                       target.width(), target.height());
    painter.drawPixmap(target, scaled, source);

    const QColor base = widget->palette().color(QPalette::Window);
    const bool dark = base.lightness() < 128;
    // On a light theme the photo has to fade into the window colour much
    // stronger, otherwise the dim subtitle (placeholder grey) disappears on
    // the washed-out image (this is the GNOME/Adwaita case).
    QLinearGradient gradient(0, 0, 0, target.height());
    gradient.setColorAt(0.0, QColor(base.red(), base.green(), base.blue(), dark ? 96 : 175));
    gradient.setColorAt(0.5, QColor(base.red(), base.green(), base.blue(), dark ? 190 : 224));
    gradient.setColorAt(1.0, QColor(base.red(), base.green(), base.blue(), dark ? 246 : 250));
    painter.fillRect(target, gradient);
    if (dark) painter.fillRect(target, QColor(0, 0, 0, 58));
}

}  // namespace theme
