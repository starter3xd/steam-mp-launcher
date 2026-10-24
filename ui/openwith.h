#pragma once

#include <QDialog>
#include <QJsonObject>

class Backend;
class QLabel;

/// "Open with" card: add the .exe to the current game's launchers/sidecars,
/// or run it once.  Used by the desktop entry and `--open`.
class OpenWithDialog : public QDialog {
    Q_OBJECT
public:
    OpenWithDialog(Backend* backend, const QString& file, QWidget* parent = nullptr);
    bool runOnce() const { return runOnce_; }
    QString appId() const { return appid_; }

private:
    void addTo(const QString& which);
    QString labelFor(const QString& path) const;

    Backend* backend_ = nullptr;
    QString file_;
    QJsonObject state_;
    bool runOnce_ = false;
    QString appid_;
    QLabel* status_ = nullptr;
};
