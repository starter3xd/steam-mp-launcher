#include "theme.h"

#include <QApplication>
#include <QCoreApplication>
#include <QFileInfo>
#include <QIcon>
#include <QFontMetrics>
#include <QLinearGradient>
#include <QPainter>
#include <QPixmap>
#include <QStyle>

namespace theme {

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

QIcon icon(const QString& name, int size) {
    QIcon result = QIcon::fromTheme(name);
    if (result.isNull()) {
        // GNOME/Adwaita ships many icons only as `*-symbolic`: monochrome
        // glyphs that Qt does not tint by itself, so paint them with the
        // text colour (otherwise they vanish on light or dark themes).
        const QIcon symbolic = QIcon::fromTheme(name + QStringLiteral("-symbolic"));
        if (!symbolic.isNull()) {
            QImage image = symbolic.pixmap(size, size).toImage().convertToFormat(QImage::Format_ARGB32);
            QPainter painter(&image);
            painter.setCompositionMode(QPainter::CompositionMode_SourceIn);
            painter.fillRect(image.rect(), QApplication::palette().color(QPalette::WindowText));
            painter.end();
            result = QIcon(QPixmap::fromImage(image));
        }
    }
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
