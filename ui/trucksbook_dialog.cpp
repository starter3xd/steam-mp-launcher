#include "trucksbook_dialog.h"
#include "i18n.h"

#include "backend.h"
#include "theme.h"

#include <QApplication>
#include <QClipboard>
#include <QDir>
#include <QFileDialog>
#include <QFileInfo>
#include <QHBoxLayout>
#include <QLabel>
#include <QProcess>
#include <QProcessEnvironment>
#include <QProgressBar>
#include <QPushButton>
#include <QTimer>
#include <QVBoxLayout>

static const char kTrucksBookDir[] = R"(C:\Program Files (x86)\TrucksBook Client)";

TrucksBookDialog::TrucksBookDialog(Backend* backend, const QString& appid, QWidget* parent)
    : QDialog(parent), backend_(backend), appid_(appid) {
    state_ = backend_->call("picker", appid_.isEmpty() ? QJsonObject{}
                                                       : QJsonObject{{"appid", appid_}});
    appid_ = state_.value("game").toObject().value("appid").toString(appid_);
    setWindowTitle(i18n("Install TrucksBook") + QStringLiteral(" · Steam MP Launcher"));
    setMinimumWidth(540);
    buildUi();
    detect();
}

TrucksBookDialog::~TrucksBookDialog() {
    if (proc_ && proc_->state() != QProcess::ProcessState::NotRunning) proc_->kill();
}

void TrucksBookDialog::buildUi() {
    auto* root = new QVBoxLayout(this);
    root->setContentsMargins(18, 16, 18, 14);
    root->setSpacing(8);

    auto* title = new QLabel(i18n("Install TrucksBook"), this);
    title->setFont(theme::boldFont(title->font(), 2));
    root->addWidget(title);

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

    foundLabel_ = new QLabel(this);
    foundLabel_->setTextFormat(Qt::PlainText);
    theme::recolor(foundLabel_, theme::positive());
    foundLabel_->hide();
    root->addWidget(foundLabel_);

    auto* setupRow = new QHBoxLayout();
    setupRow->setSpacing(8);
    setupTitle_ = new QLabel(i18n("TrucksBook setup"), this);
    setupRow->addWidget(setupTitle_);
    fileLabel_ = new QLabel(i18n("No file"), this);
    fileLabel_->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
    setupRow->addWidget(fileLabel_, 1);
    choose_ = new QPushButton(i18n("Choose file"), this);
    setupRow->addWidget(choose_);
    root->addLayout(setupRow);

    gameFoundLabel_ = new QLabel(this);
    theme::recolor(gameFoundLabel_, theme::positive());
    gameFoundLabel_->hide();
    root->addWidget(gameFoundLabel_);

    auto* gameRow = new QHBoxLayout();
    gameRow->setSpacing(8);
    gameTitle_ = new QLabel(i18n("Game .exe"), this);
    gameRow->addWidget(gameTitle_);
    gameLabel_ = new QLabel(i18n("Not chosen"), this);
    gameLabel_->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
    gameRow->addWidget(gameLabel_, 1);
    gameChoose_ = new QPushButton(i18n("Choose file"), this);
    gameRow->addWidget(gameChoose_);
    copy_ = new QPushButton(i18n("Copy path"), this);
    gameRow->addWidget(copy_);
    root->addLayout(gameRow);

    hint_ = new QLabel(
        i18n("The TrucksBook setup asks for the game .exe and cannot install "
           "silently. Run the installer, then paste the path above into its "
           "Browse dialog."),
        this);
    hint_->setWordWrap(true);
    theme::dim(hint_);
    root->addWidget(hint_);

    error_ = new QLabel(this);
    error_->setWordWrap(true);
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
    add_ = new QPushButton(i18n("Add to sidecars"), this);
    add_->hide();
    buttons->addWidget(add_);
    run_ = new QPushButton(i18n("Run installer"), this);
    run_->setDefault(true);
    buttons->addWidget(run_);
    root->addLayout(buttons);

    connect(choose_, &QPushButton::clicked, this, [this](bool) { chooseInstaller(); });
    connect(gameChoose_, &QPushButton::clicked, this, [this](bool) { chooseGameExe(); });
    connect(copy_, &QPushButton::clicked, this, [this](bool) { copyGamePath(); });
    connect(run_, &QPushButton::clicked, this, [this](bool) { runInstaller(); });
    connect(add_, &QPushButton::clicked, this, [this](bool) { addClient(); });
    connect(close_, &QPushButton::clicked, this, [this](bool) { accept(); });
    connect(cancel_, &QPushButton::clicked, this, [this](bool) { reject(); });
}

void TrucksBookDialog::detect() {
    const QJsonObject info = backend_->call("install_detect", {
        {"appid", appid_},
        {"kind", "trucksbook"},
        {"compatdata", compatdata()},
        {"game_exe", state_.value("game").toObject().value("exe").toString()},
    });
    downloads_ = info.value("downloads").toString();
    steamRoot_ = info.value("steam_root").toString();
    prefix_ = info.value("prefix").toString();
    hasS_ = info.value("has_s").toBool();
    clientPath_ = info.value("client").toString();
    installerPath_ = info.value("installer").toString();
    gameExe_ = info.value("game_exe").toString();
    gameExeWin_ = info.value("game_exe_win").toString();

    if (!installerPath_.isEmpty()) {
        fileLabel_->setText(QFileInfo(installerPath_).fileName());
        fileLabel_->setToolTip(installerPath_);
    }
    if (!gameExe_.isEmpty()) {
        gameLabel_->setText(QFileInfo(gameExe_).fileName());
        gameLabel_->setToolTip(gameExeWin_);
        gameFoundLabel_->setText(i18n("Game .exe found automatically."));
        gameFoundLabel_->show();
    }
    if (!clientPath_.isEmpty()) {
        foundLabel_->setText(i18n("TrucksBook is already installed."));
        foundLabel_->show();
        add_->show();
    } else if (!installerPath_.isEmpty()) {
        foundLabel_->setText(i18n("Found automatically in Downloads."));
        foundLabel_->show();
    }
    updateAction();
}

void TrucksBookDialog::updateAction() {
    copy_->setEnabled(!gameExeWin_.isEmpty());
    if (!clientPath_.isEmpty()) {
        action_->setText(i18n("TrucksBook is already installed."));
        run_->hide();
        add_->show();
        showSourceRows(false);
    } else if (!installerPath_.isEmpty()) {
        action_->setText(i18n("Run the installer and choose the game .exe."));
        run_->show();
        run_->setEnabled(true);
        showSourceRows(true);
    } else {
        action_->setText(i18n("Choose the TrucksBook setup."));
        run_->setEnabled(false);
        run_->show();
        showSourceRows(true);
    }
}

void TrucksBookDialog::showSourceRows(bool visible) {
    for (QWidget* widget : {static_cast<QWidget*>(setupTitle_),
                            static_cast<QWidget*>(fileLabel_),
                            static_cast<QWidget*>(choose_),
                            static_cast<QWidget*>(gameTitle_),
                            static_cast<QWidget*>(gameLabel_),
                            static_cast<QWidget*>(gameChoose_),
                            static_cast<QWidget*>(copy_),
                            static_cast<QWidget*>(hint_)}) {
        widget->setVisible(visible);
    }
    if (!visible) gameFoundLabel_->hide();
}

void TrucksBookDialog::chooseInstaller() {
    QString start = downloads_;
    if (!QDir(start).exists()) start = QDir::homePath();
    const QString path = QFileDialog::getOpenFileName(
        this, i18n("Choose the TrucksBook setup"), start,
        i18n("TrucksBook setup (*.exe);;All files (*)"));
    if (path.isEmpty()) return;
    installerPath_ = path;
    fileLabel_->setText(QFileInfo(path).fileName());
    fileLabel_->setToolTip(path);
    foundLabel_->hide();
    updateAction();
}

QString TrucksBookDialog::windowsPath(const QString& path) const {
    if (path.isEmpty()) return {};
    const QFileInfo info(path);
    const QString absolute = info.canonicalFilePath().isEmpty() ? info.absoluteFilePath()
                                                               : info.canonicalFilePath();
    if (hasS_ && !steamRoot_.isEmpty()) {
        QString root = QFileInfo(steamRoot_).canonicalFilePath();
        if (root.isEmpty()) root = steamRoot_;
        while (root.endsWith('/')) root.chop(1);
        if (absolute == root || absolute.startsWith(root + '/')) {
            QString relative = absolute.mid(root.size());
            while (relative.startsWith('/')) relative.remove(0, 1);
            relative.replace('/', '\\');
            return QStringLiteral("S:\\") + relative;
        }
    }
    QString rest = absolute;
    while (rest.startsWith('/')) rest.remove(0, 1);
    rest.replace('/', '\\');
    return QStringLiteral("Z:\\") + rest;
}

void TrucksBookDialog::chooseGameExe() {
    QString start = gameExe_.isEmpty() ? QString() : QFileInfo(gameExe_).absolutePath();
    if (start.isEmpty() && !steamRoot_.isEmpty()) {
        start = QDir(steamRoot_).filePath("steamapps/common");
    }
    if (!QDir(start).exists()) start = QDir::homePath();
    const QString path = QFileDialog::getOpenFileName(
        this, i18n("Choose the game .exe"), start, i18n("Game (*.exe)"));
    if (path.isEmpty()) return;
    gameExe_ = path;
    gameExeWin_ = windowsPath(path);
    gameLabel_->setText(QFileInfo(path).fileName());
    gameLabel_->setToolTip(gameExeWin_);
    gameFoundLabel_->hide();
    updateAction();
}

void TrucksBookDialog::copyGamePath() {
    if (gameExeWin_.isEmpty()) return;
    QApplication::clipboard()->setText(gameExeWin_);
    copy_->setText(i18n("Copied"));
    QTimer::singleShot(1500, this, [this] { copy_->setText(i18n("Copy path")); });
}

void TrucksBookDialog::runInstaller() {
    if (installerPath_.isEmpty()) return;
    const QString proton = protonPath();
    if (proton.isEmpty() || !QFileInfo(proton).isFile()) {
        error_->setText(i18n("No Proton found."));
        error_->show();
        return;
    }
    const QString data = compatdata();
    if (data.isEmpty() || !QDir(data).exists()) {
        error_->setText(i18n("Run the game once first."));
        error_->show();
        return;
    }
    if (!QFileInfo(installerPath_).isFile()) {
        error_->setText(i18n("The installer file is missing."));
        error_->show();
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
    proc_->setWorkingDirectory(QFileInfo(installerPath_).absolutePath());
    proc_->setProgram(proton);
    proc_->setArguments({"runinprefix", installerPath_, "/NORESTART", "/SP-",
                         QStringLiteral("/DIR=%1").arg(kTrucksBookDir)});
    connect(proc_, QOverload<int, QProcess::ExitStatus>::of(&QProcess::finished), this,
            &TrucksBookDialog::finished);
    connect(proc_, &QProcess::errorOccurred, this, &TrucksBookDialog::failed);

    choose_->hide();
    gameChoose_->hide();
    add_->hide();
    run_->hide();
    cancel_->show();
    progress_->show();
    action_->setText(i18n("The installer is open. Choose the game .exe and finish the setup."));
    error_->hide();
    proc_->start();
}

void TrucksBookDialog::finished(int code, QProcess::ExitStatus status) {
    if (cancelled_) return;
    progress_->hide();
    choose_->show();
    gameChoose_->show();
    if (status != QProcess::NormalExit || code != 0) {
        failedInstall(i18n("Installer exited with code %1.", code));
        return;
    }
    addClient(true);
}

void TrucksBookDialog::failed(QProcess::ProcessError error) {
    if (cancelled_) return;
    progress_->hide();
    failedInstall(i18n("Error %1.", static_cast<int>(error)));
}

void TrucksBookDialog::failedInstall(const QString& message) {
    error_->setText(message);
    error_->show();
    action_->setText(i18n("Installer closed."));
    choose_->show();
    gameChoose_->show();
    cancel_->show();
    run_->show();
    run_->setEnabled(true);
    showSourceRows(true);
}

void TrucksBookDialog::addClient(bool afterInstall) {
    const QJsonObject reply = backend_->call("install_finish", {
        {"appid", appid_},
        {"kind", "trucksbook"},
        {"compatdata", compatdata()},
        {"game_exe", state_.value("game").toObject().value("exe").toString()},
    });
    if (!Backend::ok(reply)) {
        error_->setText(i18n("Could not save the sidecar list."));
        error_->show();
        return;
    }
    if (!reply.value("found").toBool()) {
        error_->setText(afterInstall ? i18n("TB Client.exe was not found after installation.")
                                     : i18n("TB Client.exe was not found."));
        error_->show();
        return;
    }
    applyResult(reply);
}

void TrucksBookDialog::applyResult(const QJsonObject& reply) {
    addedPath_ = reply.value("path").toString();
    progress_->hide();
    showSourceRows(false);
    add_->hide();
    run_->hide();
    cancel_->hide();
    close_->show();
    close_->setDefault(true);
    action_->setText(reply.value("added").toBool() ? i18n("Added to sidecars.")
                                                   : i18n("Already added to sidecars."));
    error_->hide();
}

QString TrucksBookDialog::compatdata() const {
    return state_.value("game").toObject().value("compatdata").toString();
}

QString TrucksBookDialog::protonPath() const {
    return state_.value("game").toObject().value("proton").toString();
}

void TrucksBookDialog::reject() {
    cancelled_ = true;
    if (proc_ && proc_->state() != QProcess::ProcessState::NotRunning) proc_->kill();
    QDialog::reject();
}
