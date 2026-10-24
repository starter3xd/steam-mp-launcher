#pragma once

#include <QDialog>
#include <QJsonObject>
#include <QProcess>

class Backend;
class QLabel;
class QProgressBar;
class QPushButton;

/// Guided TrucksBook install, ported 1:1 from `mpgui/ui/trucksbook.py`.
///
/// The Inno Setup wizard cannot install silently: it has a custom "Select
/// Location of the Games" page.  The dialog finds the setup and the game .exe,
/// shows the Windows path to paste into the wizard and adds `TB Client.exe`
/// to the sidecar list once the setup is done.
class TrucksBookDialog : public QDialog {
    Q_OBJECT
public:
    TrucksBookDialog(Backend* backend, const QString& appid, QWidget* parent = nullptr);
    ~TrucksBookDialog() override;

    QString addedPath() const { return addedPath_; }

protected:
    void reject() override;

private:
    void buildUi();
    void detect();
    void updateAction();
    void showSourceRows(bool visible);
    void chooseInstaller();
    void chooseGameExe();
    void copyGamePath();
    void runInstaller();
    void finished(int code, QProcess::ExitStatus status);
    void failed(QProcess::ProcessError error);
    void failedInstall(const QString& message);
    void addClient(bool afterInstall = false);
    void applyResult(const QJsonObject& reply);

    QString compatdata() const;
    QString protonPath() const;
    QString windowsPath(const QString& path) const;

    Backend* backend_ = nullptr;
    QString appid_;
    QJsonObject state_;

    QString installerPath_;
    QString clientPath_;
    QString gameExe_;
    QString gameExeWin_;
    QString downloads_;
    QString steamRoot_;
    QString prefix_;
    bool hasS_ = false;

    QProcess* proc_ = nullptr;
    bool cancelled_ = false;
    QString addedPath_;

    QLabel* action_ = nullptr;
    QProgressBar* progress_ = nullptr;
    QLabel* foundLabel_ = nullptr;
    QLabel* setupTitle_ = nullptr;
    QLabel* fileLabel_ = nullptr;
    QPushButton* choose_ = nullptr;
    QLabel* gameFoundLabel_ = nullptr;
    QLabel* gameTitle_ = nullptr;
    QLabel* gameLabel_ = nullptr;
    QPushButton* gameChoose_ = nullptr;
    QPushButton* copy_ = nullptr;
    QLabel* hint_ = nullptr;
    QLabel* error_ = nullptr;
    QPushButton* close_ = nullptr;
    QPushButton* cancel_ = nullptr;
    QPushButton* add_ = nullptr;
    QPushButton* run_ = nullptr;
};
