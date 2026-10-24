#pragma once

#include <QColor>
#include <QFont>
#include <QIcon>
#include <QLabel>
#include <QPalette>
#include <QPixmap>
#include <QWidget>

class QFontMetrics;
class QPainter;
class QString;

namespace theme {

/// Follow the desktop's colour scheme when the platform theme cannot report
/// it: GNOME (and the bundle, which ships no GTK platform plugin) exposes dark
/// mode only through the XDG portal / gsettings, so the palette is switched by
/// hand instead of leaving a white window on a dark desktop.
/// `STEAM_MP_THEME=dark` or `=light` overrides the detection.
void applySystemTheme();
/// Multi-line description of what was detected (`--theme-info`).
QString systemThemeReport();

QColor dim();
QColor negative();
QColor positive();
QColor neutral();

void recolor(QWidget* widget, const QColor& color,
             QPalette::ColorRole role = QPalette::WindowText);
QLabel* dim(QLabel* label);
QFont boldFont(const QFont& base, qreal delta = 0.0);
QIcon icon(const QString& name, int size = 16);
/// The application icon: icon theme first, then the SVG next to the binary,
/// then a generic games icon (icons may not be rescanned right after install).
QIcon appIcon();
/// Icon extracted from a Windows .exe (empty path / broken file -> fallback).
QPixmap fileIcon(const QString& path, int size = 24);
QString elideMiddle(const QFontMetrics& metrics, const QString& text, int width);
void paintBanner(QWidget* widget, QPainter& painter, const QString& path);

}  // namespace theme
