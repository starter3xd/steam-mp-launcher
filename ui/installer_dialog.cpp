#include "installer_dialog.h"

#include "i18n.h"

#include "backend.h"
#include "notify.h"
#include "theme.h"

#include <QApplication>
#include <QDateTime>
#include <QDesktopServices>
#include <QDir>
#include <QFileDialog>
#include <QFileInfo>
#include <QFont>
#include <QHBoxLayout>
#include <QLabel>
#include <QProcess>
#include <QProcessEnvironment>
#include <QProgressBar>
#include <QPushButton>
#include <QStandardPaths>
#include <QTimer>
#include <QUrl>
#include <QVBoxLayout>

static const char kDownloadUrl[] = "https://truckersmp.com/download/zip";

static QLabel* stepLabel(QWidget* parent) {
    auto* label = new QLabel(parent);
    label->setTextFormat(Qt::PlainText);
    return label;
}

InstallerDialog::InstallerDialog(Backend* backend, const QString& appid, QWidget* parent)
    : QDialog(parent), backend_(backend), appid_(appid) {
    state_ = backend_->call("picker", appid_.isEmpty() ? QJsonObject{}
                                                       : QJsonObject{{"appid", appid_}});
    appid_ = state_.value("game").toObject().value("appid").toString(appid_);
    setWindowTitle(i18n("Install TruckersMP") + QStringLiteral(" · Steam MP Launcher"));
    setMinimumWidth(520);
    buildUi();

    const QJsonObject info = backend_->call("install_detect", {
        {"appid", appid_},
        {"kind", "truckersmp"},
        {"compatdata", compatdata()},
        {"game_exe", gameExe_},
    });
    downloads_ = info.value("downloads").toString();
    steamRoot_ = info.value("steam_root").toString();
    gameExe_ = info.value("game_exe").toString(gameExe_);
    existingPath_ = info.value("launcher").toString();
    version_ = info.value("version").toString();

    if (info.value("installed").toBool() && !existingPath_.isEmpty()) {
        // A complete Squirrel install is already in this prefix: offer to add
        // it, no setup.
        detectTimer_->stop();
        readyTimer_->stop();
        setSteps(Added);
        foundLabel_->setText(version_.isEmpty()
                                 ? i18n("TruckersMP is already installed.")
                                 : i18n("TruckersMP %1 is already installed.", version_));
        foundLabel_->show();
        setAction(i18n("Add it to the launchers."));
        open_->hide();
        choose_->hide();
        fileLabel_->hide();
        progress_->hide();
        showButtons(false, false, false, false, true, false);
    } else {
        detectDownload();
        detectTimer_->start();
    }
}

InstallerDialog::~InstallerDialog() {
    if (detectTimer_) detectTimer_->stop();
    if (readyTimer_) readyTimer_->stop();
    if (proc_ && proc_->state() != QProcess::ProcessState::NotRunning) proc_->kill();
    cleanupTemp();
}

void InstallerDialog::buildUi() {
    auto* root = new QVBoxLayout(this);
    root->setContentsMargins(18, 16, 18, 14);
    root->setSpacing(8);

    auto* title = new QLabel(i18n("Install TruckersMP"), this);
    title->setFont(theme::boldFont(title->font(), 2));
    root->addWidget(title);

    static const char* kSteps[] = {QT_TR_NOOP("Download"), QT_TR_NOOP("Run"),
                                   QT_TR_NOOP("Finish"), QT_TR_NOOP("Added")};
    auto* steps = new QVBoxLayout();
    steps->setSpacing(1);
    for (size_t i = 0; i < sizeof(kSteps) / sizeof(kSteps[0]); ++i) {
        auto* label = stepLabel(this);
        steps->addWidget(label);
        steps_.append(label);
    }
    root->addLayout(steps);

    action_ = new QLabel(this);
    action_->setWordWrap(true);
    action_->setMinimumHeight(48);
    action_->setStyleSheet(
        "QLabel { background: palette(alternate-base);"
        " border: 1px solid palette(mid);"
        " border-left: 4px solid palette(highlight);"
        " border-radius: 4px; padding: 10px 12px; }");
    action_->setFont(theme::boldFont(action_->font(), 1.5));
    root->addWidget(action_);

    progress_ = new QProgressBar(this);
    progress_->setRange(0, 0);
    progress_->setTextVisible(false);
    progress_->setFixedHeight(6);
    progress_->hide();
    root->addWidget(progress_);

    auto* fileRow = new QHBoxLayout();
    fileRow->setSpacing(8);
    open_ = new QPushButton(i18n("Download page"), this);
    open_->setToolTip(i18n("truckersmp.com/download/zip (account required)"));
    fileRow->addWidget(open_);
    fileLabel_ = new QLabel(i18n("No file"), this);
    fileLabel_->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
    fileRow->addWidget(fileLabel_, 1);
    choose_ = new QPushButton(i18n("Choose file"), this);
    fileRow->addWidget(choose_);
    root->addLayout(fileRow);

    foundLabel_ = new QLabel(this);
    foundLabel_->setTextFormat(Qt::PlainText);
    theme::recolor(foundLabel_, theme::positive());
    foundLabel_->hide();
    root->addWidget(foundLabel_);

    error_ = new QLabel(this);
    error_->setWordWrap(true);
    error_->setTextInteractionFlags(Qt::TextSelectableByMouse);
    theme::recolor(error_, theme::negative());
    error_->hide();
    root->addWidget(error_);

    auto* buttons = new QHBoxLayout();
    buttons->setSpacing(8);
    buttons->addStretch(1);
    close_ = new QPushButton(i18n("Close"), this);
    close_->hide();
    buttons->addWidget(close_);
    cancel_ = new QPushButton(i18n("Cancel"), this);
    buttons->addWidget(cancel_);
    search_ = new QPushButton(i18n("Search again"), this);
    search_->hide();
    buttons->addWidget(search_);
    manual_ = new QPushButton(i18n("Choose .exe"), this);
    manual_->hide();
    buttons->addWidget(manual_);
    add_ = new QPushButton(i18n("Add to launchers"), this);
    add_->hide();
    buttons->addWidget(add_);
    continue_ = new QPushButton(i18n("Close launcher and finish"), this);
    continue_->hide();
    buttons->addWidget(continue_);
    keep_ = new QPushButton(i18n("Keep it open"), this);
    keep_->hide();
    buttons->addWidget(keep_);
    run_ = new QPushButton(i18n("Run"), this);
    run_->setDefault(true);
    run_->setEnabled(false);
    buttons->addWidget(run_);
    root->addLayout(buttons);

    connect(open_, &QPushButton::clicked, this, [this](bool) { openDownload(); });
    connect(choose_, &QPushButton::clicked, this, [this](bool) { chooseInstaller(); });
    connect(run_, &QPushButton::clicked, this, [this](bool) { runInstaller(); });
    connect(continue_, &QPushButton::clicked, this, [this](bool) { closeLauncherNow(); });
    connect(keep_, &QPushButton::clicked, this, [this](bool) { keepLauncherOpen(); });
    connect(search_, &QPushButton::clicked, this, [this](bool) { continueAfterClose(); });
    connect(manual_, &QPushButton::clicked, this, [this](bool) { chooseManually(); });
    connect(add_, &QPushButton::clicked, this, [this](bool) { addExisting(); });
    connect(close_, &QPushButton::clicked, this, [this](bool) { accept(); });
    connect(cancel_, &QPushButton::clicked, this, [this](bool) { reject(); });

    detectTimer_ = new QTimer(this);
    detectTimer_->setInterval(1000);
    connect(detectTimer_, &QTimer::timeout, this, &InstallerDialog::detectDownload);
    readyTimer_ = new QTimer(this);
    readyTimer_->setInterval(700);
    connect(readyTimer_, &QTimer::timeout, this, &InstallerDialog::pollInstall);
    closeTimer_ = new QTimer(this);
    closeTimer_->setInterval(1000);
    connect(closeTimer_, &QTimer::timeout, this, &InstallerDialog::tickCloseCountdown);

    setSteps(Download);
    setAction(i18n("Download the installer."));
    showButtons(true);
}

void InstallerDialog::setSteps(int current) {
    static const char* kSteps[] = {QT_TR_NOOP("Download"), QT_TR_NOOP("Run"),
                                   QT_TR_NOOP("Finish"), QT_TR_NOOP("Added")};
    for (int index = 0; index < steps_.size(); ++index) {
        QLabel* label = steps_[index];
        const QString text = i18n(kSteps[index]);
        QFont font = label->font();
        if (index < current) {
            label->setText(QStringLiteral("✓ ") + text);
            font.setStrikeOut(true);
            font.setBold(false);
            theme::recolor(label, palette().color(QPalette::PlaceholderText));
        } else if (index == current) {
            label->setText(QStringLiteral("→ ") + text);
            font.setStrikeOut(false);
            font.setBold(true);
            theme::recolor(label, palette().color(QPalette::Highlight));
        } else {
            label->setText(QStringLiteral("○ ") + text);
            font.setStrikeOut(false);
            font.setBold(false);
            theme::recolor(label, palette().color(QPalette::WindowText));
        }
        label->setFont(font);
    }
}

void InstallerDialog::setAction(const QString& text) {
    action_->setText(text);
    error_->hide();
}

void InstallerDialog::setError(const QString& text) {
    error_->setText(text);
    error_->setVisible(!text.isEmpty());
}

void InstallerDialog::showButtons(bool run, bool resume, bool search, bool manual,
                                  bool add, bool close) {
    run_->setVisible(run);
    continue_->setVisible(resume);
    keep_->setVisible(false);
    search_->setVisible(search);
    manual_->setVisible(manual);
    add_->setVisible(add);
    close_->setVisible(close);
    cancel_->setVisible(!close);
    if (add) {
        add_->setDefault(true);
    } else if (resume) {
        continue_->setDefault(true);
    } else if (close) {
        close_->setDefault(true);
    } else {
        run_->setDefault(true);
    }
}

void InstallerDialog::detectDownload() {
    if (started_ || manualChoice_) return;
    const QJsonObject reply = backend_->call("install_find_installer", {{"kind", "truckersmp"}});
    const QString path = reply.value("path").toString();
    if (!path.isEmpty() && path != installerPath_) {
        detectTimer_->stop();
        setInstaller(path, true);
    }
}

void InstallerDialog::setInstaller(const QString& path, bool detected) {
    installerPath_ = path;
    fileLabel_->setText(QFileInfo(path).fileName());
    fileLabel_->setToolTip(path);
    fileLabel_->show();
    run_->setEnabled(true);
    setSteps(Run);
    setAction(i18n("Run the installer."));
    showButtons(true);
    error_->hide();
    foundLabel_->setText(detected ? i18n("Found automatically in Downloads.") : QString());
    foundLabel_->setVisible(detected);
}

void InstallerDialog::chooseInstaller() {
    QString start = downloads_;
    if (!QDir(start).exists()) start = QDir::homePath();
    const QString path = QFileDialog::getOpenFileName(
        this, i18n("Choose exe file"), start,
        i18n("TruckersMP installer (*.exe *.zip);;All files (*)"));
    if (path.isEmpty()) return;
    manualChoice_ = true;
    detectTimer_->stop();
    setInstaller(path, false);
}

void InstallerDialog::openDownload() {
    if (!QDesktopServices::openUrl(QUrl::fromUserInput(kDownloadUrl))) {
        setError(i18n("Could not open the browser."));
        return;
    }
    if (!manualChoice_) detectTimer_->start();
}

void InstallerDialog::runInstaller() {
    if (installerPath_.isEmpty()) return;

    const QString proton = protonPath();
    if (proton.isEmpty() || !QFileInfo(proton).isFile()) {
        setSteps(Run);
        setAction(i18n("No Proton found."));
        return;
    }
    const QString data = compatdata();
    if (data.isEmpty() || !QDir(data).exists()) {
        setSteps(Run);
        setAction(i18n("Run ETS2 once first."));
        return;
    }

    started_ = true;
    detectTimer_->stop();
    readyTimer_->stop();
    finishing_ = false;
    exitCode_ = 0;
    keepOpen_ = false;
    closeCountdown_ = 0;
    closeTimer_->stop();
    since_ = QDateTime::currentSecsSinceEpoch();
    progress_->show();
    setAction(i18n("Unpacking the installer."));

    const QJsonObject prep = backend_->call("install_prepare", {{"path", installerPath_}});
    if (!Backend::ok(prep)) {
        started_ = false;
        progress_->hide();
        setError(i18n("Could not unpack the file: %1", Backend::error(prep)));
        return;
    }
    cleanupTemp();
    tempDir_ = prep.value("temp_dir").toString();
    const QString exe = prep.value("exe").toString();
    if (exe.isEmpty()) {
        started_ = false;
        progress_->hide();
        setSteps(Run);
        setAction(i18n("Run the installer."));
        setError(i18n("No .exe installer was found."));
        return;
    }

    QProcessEnvironment env = QProcessEnvironment::systemEnvironment();
    env.remove("LD_LIBRARY_PATH");
    env.remove("LD_PRELOAD");
    env.remove("LC_ALL");
    env.insert("STEAM_COMPAT_DATA_PATH", data);
    if (!steamRoot_.isEmpty()) env.insert("STEAM_COMPAT_CLIENT_INSTALL_PATH", steamRoot_);

    proc_ = new QProcess(this);
    proc_->setProcessEnvironment(env);
    proc_->setWorkingDirectory(QFileInfo(exe).absolutePath());
    proc_->setProgram(proton);
    proc_->setArguments({"runinprefix", exe});
    connect(proc_, QOverload<int, QProcess::ExitStatus>::of(&QProcess::finished), this,
            &InstallerDialog::installerFinished);
    connect(proc_, &QProcess::errorOccurred, this, &InstallerDialog::installerFailed);

    open_->hide();
    choose_->hide();
    fileLabel_->hide();
    foundLabel_->hide();
    progress_->show();
    setSteps(Run);
    setAction(i18n("Wait for the installer."));
    showButtons();
    proc_->start();
    readyTicks_ = 0;
    readyTimer_->start();
}

void InstallerDialog::installerFinished(int code, QProcess::ExitStatus status) {
    if (cancelled_) return;
    // The setup can exit before Squirrel has written `app.ico` and its final
    // log line, so the probe decides whether the install is complete; a
    // non-zero code is only reported if the probe never sees a full tree.
    exitCode_ = status == QProcess::NormalExit ? code : -1;
    progress_->show();
    setSteps(Finish);
    setAction(i18n("Finishing the installation…"));
    error_->hide();
    foundLabel_->hide();
    showButtons();
    readyTicks_ = 0;
    readyTimer_->start();
    pollInstall();
}

void InstallerDialog::installerFailed(QProcess::ProcessError error) {
    if (cancelled_) return;
    showInstallerError(i18n("Could not start the installer (error %1).", static_cast<int>(error)));
}

void InstallerDialog::showInstallerError(const QString& message) {
    if (cancelled_) return;
    started_ = false;
    readyTimer_->stop();
    progress_->hide();
    open_->show();
    choose_->show();
    fileLabel_->show();
    foundLabel_->setVisible(!foundLabel_->text().isEmpty());
    setSteps(Run);
    setAction(i18n("Installer failed."));
    setError(message);
    run_->setEnabled(true);
    showButtons(true);
}

void InstallerDialog::pollInstall() {
    if (cancelled_ || finishing_ || probing_) return;
    probing_ = true;
    struct Guard {
        bool& flag;
        ~Guard() { flag = false; }
    } guard{probing_};
    const bool setupRunning = proc_ && proc_->state() != QProcess::ProcessState::NotRunning;
    const QJsonObject reply = backend_->call("install_probe", {
        {"appid", appid_},
        {"kind", "truckersmp"},
        {"compatdata", compatdata()},
        {"game_exe", gameExe_},
        {"since", since_},
    });
    const QString version = reply.value("version").toString();
    if (!version.isEmpty()) version_ = version;

    // `installing` covers the Squirrel tree in the prefix (Update.exe is
    // writing app-<ver>), `setupRunning` the UI part before it.
    if (reply.value("installing").toBool() || setupRunning) {
        readyTicks_ = 0;
        // Squirrel's hook wants 15 s of silence before it finishes; a second
        // counter keeps the window visibly alive during that wait.
        const qint64 elapsed = QDateTime::currentSecsSinceEpoch() - since_;
        setAction(version_.isEmpty()
                      ? i18n("Finishing the installation… (%1 s)", elapsed)
                      : i18n("Finishing the installation… (v%1, %2 s)", version_, elapsed));
        return;
    }
    if (!reply.value("complete").toBool()) {
        // A partial tree (stub only) looks installed for ~17 s, so wait for
        // the real marker instead of trusting the .exe.
        ++readyTicks_;
        if (readyTicks_ == 1) setAction(i18n("Waiting for the files…"));
        if (readyTicks_ > 25) {
            showInstallerError(exitCode_ == 0
                                   ? i18n("The installer did not finish. Run it again.")
                                   : i18n("The installer exited with code %1.", exitCode_));
        }
        return;
    }

    readyTicks_ = 0;
    closePolls_ = 0;
    foundLabel_->setText(version_.isEmpty()
                             ? i18n("TruckersMP is installed.")
                             : i18n("TruckersMP %1 is installed.", version_));
    foundLabel_->show();
    if (reply.value("running").toBool()) {
        // Squirrel's first run leaves the launcher open *in front of* this
        // wizard; close it automatically after a short grace period unless
        // the user wants to keep it.
        updateCloseCard();
        if (++closePolls_ % 14 == 0) {
            raise();
            QApplication::alert(this, 0);
        }
        return;
    }
    readyTimer_->stop();
    continueAfterClose();
}

void InstallerDialog::askToCloseLauncher() {
    if (askedClose_) return;
    askedClose_ = true;
    // The launcher was started by the setup and covers this dialog.  Do not
    // toggle window flags here: recreating the native window of a *modal*
    // dialog from inside its own event loop can leave the dialog stuck.
    // Raise it out-of-band and rely on the notification for discoverability.
    QTimer::singleShot(0, this, [this] {
        if (cancelled_ || finishing_) return;
        raise();
        activateWindow();
        QApplication::alert(this, 0);
    });
    notify::send(version_.isEmpty() ? i18n("TruckersMP is installed.")
                                    : i18n("TruckersMP %1 is installed.", version_),
                 i18n("Close the launcher window to finish the setup, or use "
                      "\"Close launcher and finish\" in the installer window."));
}

void InstallerDialog::updateCloseCard() {
    if (!closeTimer_->isActive() && closeCountdown_ == 0) {
        closeCountdown_ = 8;
        closeTimer_->start();
    }
    setAction(i18n("Closing the launcher in %1 s…", closeCountdown_));
    showButtons(false, true, false, false, false, false);
    continue_->setText(i18n("Close now"));
    keep_->show();
    askToCloseLauncher();
}

void InstallerDialog::tickCloseCountdown() {
    if (cancelled_ || finishing_ || keepOpen_) {
        closeTimer_->stop();
        return;
    }
    --closeCountdown_;
    if (closeCountdown_ > 0) {
        setAction(i18n("Closing the launcher in %1 s…", closeCountdown_));
        return;
    }
    closeTimer_->stop();
    closeCountdown_ = 0;
    closeLauncherNow();
}

void InstallerDialog::keepLauncherOpen() {
    // The launcher is left running; the list entry and the path are written
    // anyway, because the launcher already rewrote its options file on start.
    keepOpen_ = true;
    finishing_ = true;
    askedClose_ = false;
    closeTimer_->stop();
    closeCountdown_ = 0;
    readyTimer_->stop();
    setSteps(Added);
    setAction(i18n("Adding to the launchers…"));
    showButtons();
    QTimer::singleShot(0, this, &InstallerDialog::searchLauncher);
}

void InstallerDialog::closeLauncherNow() {
    continueAfterClose();
}

void InstallerDialog::continueAfterClose() {
    if (cancelled_ || finishing_) return;
    finishing_ = true;
    askedClose_ = false;
    closeTimer_->stop();
    closeCountdown_ = 0;
    readyTimer_->stop();
    setSteps(Added);
    setAction(i18n("Closing the launcher and adding it…"));
    showButtons();
    // Closing the launcher is the last thing that may overwrite the options
    // file: ask it to quit, then force it if it ignores the request.
    backend_->call("install_close", {
        {"appid", appid_},
        {"compatdata", compatdata()},
        {"since", since_},
        {"force", false},
    });
    QTimer::singleShot(1200, this, [this] {
        backend_->call("install_close", {
            {"appid", appid_},
            {"compatdata", compatdata()},
            {"since", since_},
            {"force", true},
        });
        QTimer::singleShot(300, this, &InstallerDialog::searchLauncher);
    });
}

void InstallerDialog::searchLauncher() {
    const QJsonObject reply = backend_->call("install_finish", {
        {"appid", appid_},
        {"kind", "truckersmp"},
        {"compatdata", compatdata()},
        {"game_exe", gameExe_},
    });
    if (!Backend::ok(reply)) {
        setError(Backend::error(reply));
        return;
    }
    if (!reply.value("found").toBool()) {
        finishing_ = false;
        setSteps(Finish);
        setAction(i18n("Launcher not found."));
        setError(i18n("Close it, then search again."));
        showButtons(false, false, true, true, false, false);
        return;
    }
    applyResult(reply);
}

void InstallerDialog::chooseManually() {
    QString start = prefix().isEmpty() ? QString() : QDir(prefix()).filePath("drive_c/users");
    if (!QDir(start).exists()) start = QDir::homePath();
    const QString path = QFileDialog::getOpenFileName(
        this, i18n("Choose TruckersMP-Launcher.exe"), start,
        i18n("Windows programs (*.exe)"));
    if (!path.isEmpty()) addPath(path);
}

void InstallerDialog::addPath(const QString& path) {
    const QJsonObject reply = backend_->call("install_finish", {
        {"appid", appid_},
        {"kind", "truckersmp"},
        {"compatdata", compatdata()},
        {"game_exe", gameExe_},
        {"path", path},
    });
    if (!Backend::ok(reply)) {
        setError(i18n("Could not save the launcher list."));
        return;
    }
    if (!reply.value("found").toBool()) {
        finishing_ = false;
        setSteps(Finish);
        setAction(i18n("Launcher not found."));
        setError(i18n("Close it, then search again."));
        showButtons(false, false, true, true, false, false);
        return;
    }
    applyResult(reply);
}

void InstallerDialog::addExisting() {
    const QJsonObject reply = backend_->call("install_finish", {
        {"appid", appid_},
        {"kind", "truckersmp"},
        {"compatdata", compatdata()},
        {"game_exe", gameExe_},
    });
    if (!Backend::ok(reply)) {
        setError(Backend::error(reply));
        return;
    }
    if (!reply.value("found").toBool()) {
        setAction(i18n("Launcher not found."));
        setError(i18n("Close it, then search again."));
        showButtons(false, false, false, false, true, false);
        return;
    }
    applyResult(reply);
}

void InstallerDialog::applyResult(const QJsonObject& reply) {
    addedPath_ = reply.value("path").toString();
    gamePathSet_ = reply.value("game_path_set").toBool();
    open_->hide();
    choose_->hide();
    fileLabel_->hide();
    foundLabel_->hide();
    add_->hide();
    progress_->hide();
    setSteps(Done);
    setAction(reply.value("added").toBool() ? i18n("Added to launchers.")
                                            : i18n("Already added."));
    if (!gamePathSet_) {
        setError(i18n("Game path not updated."));
    } else {
        error_->hide();
    }
    showButtons(false, false, false, false, false, true);
}

void InstallerDialog::cleanupTemp() {
    if (tempDir_.isEmpty()) return;
    QDir(tempDir_).removeRecursively();
    tempDir_.clear();
}

QString InstallerDialog::compatdata() const {
    return state_.value("game").toObject().value("compatdata").toString();
}

QString InstallerDialog::protonPath() const {
    return state_.value("game").toObject().value("proton").toString();
}

QString InstallerDialog::prefix() const {
    return state_.value("game").toObject().value("prefix").toString();
}

void InstallerDialog::reject() {
    cancelled_ = true;
    detectTimer_->stop();
    readyTimer_->stop();
    closeTimer_->stop();
    if (proc_ && proc_->state() != QProcess::ProcessState::NotRunning) proc_->kill();
    cleanupTemp();
    QDialog::reject();
}

void InstallerDialog::accept() {
    detectTimer_->stop();
    readyTimer_->stop();
    closeTimer_->stop();
    cleanupTemp();
    QDialog::accept();
}
