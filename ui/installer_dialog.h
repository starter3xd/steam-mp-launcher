#pragma once

#include <QDialog>
#include <QJsonObject>
#include <QProcess>
#include <QVector>

class Backend;
class QLabel;
class QProgressBar;
class QPushButton;
class QTimer;

/// Guided TruckersMP install, ported 1:1 from `mpgui/ui/installer.py`.
///
/// Download -> Run -> Finish -> Added.  The newest installer in Downloads is
/// picked up automatically.  After the setup, the wizard waits for Squirrel to
/// report a complete `app-<ver>` tree (not just the ~396 kB stub), asks to
/// close the launcher only when it is still running, writes the ETS2 path into
/// `launcher-options.json` and adds the launcher to the list.
class InstallerDialog : public QDialog {
    Q_OBJECT
public:
    InstallerDialog(Backend* backend, const QString& appid, QWidget* parent = nullptr);
    ~InstallerDialog() override;

    QString addedPath() const { return addedPath_; }

protected:
    void reject() override;
    void accept() override;

private:
    enum Step { Download = 0, Run = 1, Finish = 2, Added = 3, Done = 4 };

    void buildUi();
    void setSteps(int current);
    void setAction(const QString& text);
    void setError(const QString& text);
    void showButtons(bool run = false, bool resume = false, bool search = false,
                     bool manual = false, bool add = false, bool close = false);

    void detectDownload();
    void setInstaller(const QString& path, bool detected);
    void chooseInstaller();
    void openDownload();
    void runInstaller();
    void installerFinished(int code, QProcess::ExitStatus status);
    void installerFailed(QProcess::ProcessError error);
    void showInstallerError(const QString& message);
    void pollInstall();
    void askToCloseLauncher();
    void updateCloseCard();
    void tickCloseCountdown();
    void closeLauncherNow();
    void keepLauncherOpen();
    void continueAfterClose();
    void searchLauncher();
    void addPath(const QString& path);
    void addExisting();
    void applyResult(const QJsonObject& reply);
    void chooseManually();
    void cleanupTemp();

    QString compatdata() const;
    QString protonPath() const;
    QString prefix() const;

    Backend* backend_ = nullptr;
    QString appid_;
    QJsonObject state_;

    QString installerPath_;
    QString gameExe_;
    QString downloads_;
    QString steamRoot_;
    QString existingPath_;
    QString tempDir_;

    QProcess* proc_ = nullptr;
    QTimer* detectTimer_ = nullptr;
    QTimer* readyTimer_ = nullptr;
    QTimer* closeTimer_ = nullptr;

    qint64 since_ = 0;
    int readyTicks_ = 0;
    int exitCode_ = 0;
    int closePolls_ = 0;
    int closeCountdown_ = 0;
    bool probing_ = false;
    bool keepOpen_ = false;
    bool manualChoice_ = false;
    bool started_ = false;
    bool cancelled_ = false;
    bool finishing_ = false;
    bool askedClose_ = false;
    bool gamePathSet_ = false;
    QString version_;
    QString addedPath_;

    QVector<QLabel*> steps_;
    QLabel* action_ = nullptr;
    QProgressBar* progress_ = nullptr;
    QLabel* fileLabel_ = nullptr;
    QLabel* foundLabel_ = nullptr;
    QLabel* error_ = nullptr;
    QPushButton* open_ = nullptr;
    QPushButton* choose_ = nullptr;
    QPushButton* run_ = nullptr;
    QPushButton* continue_ = nullptr;
    QPushButton* keep_ = nullptr;
    QPushButton* search_ = nullptr;
    QPushButton* manual_ = nullptr;
    QPushButton* add_ = nullptr;
    QPushButton* close_ = nullptr;
    QPushButton* cancel_ = nullptr;
};
