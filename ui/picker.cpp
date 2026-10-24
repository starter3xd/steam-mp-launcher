#include "picker.h"
#include "i18n.h"

#include "backend.h"
#include "theme.h"

#include <QCheckBox>
#include <QDir>
#include <QFileDialog>
#include <QFileInfo>
#include <QFrame>
#include <QHBoxLayout>
#include <QInputDialog>
#include <QJsonArray>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
#include <QMenu>
#include <QPainter>
#include <QPushButton>
#include <QResizeEvent>
#include <QShortcut>
#include <QToolButton>
#include <QVBoxLayout>

namespace {

class ElidedLabel : public QLabel {
public:
    explicit ElidedLabel(QWidget* parent = nullptr) : QLabel(parent) {}
    void setFullText(const QString& text) {
        full_ = text;
        setToolTip(text);
        updateText();
    }
protected:
    void resizeEvent(QResizeEvent* event) override {
        QLabel::resizeEvent(event);
        updateText();
    }
private:
    void updateText() {
        setText(theme::elideMiddle(fontMetrics(), full_, qMax(0, width())));
    }
    QString full_;
};

}  // namespace

class PickRow : public QWidget {
public:
    PickRow(const QString& title, const QString& path, bool removable, bool missing,
            bool remembered, const QPixmap& rowIcon, QWidget* parent = nullptr)
        : QWidget(parent), remembered_(remembered) {
        auto* lay = new QHBoxLayout(this);
        lay->setContentsMargins(10, 5, 10, 5);
        lay->setSpacing(10);

        auto* icon = new QLabel(this);
        icon->setFixedSize(28, 28);
        icon->setAlignment(Qt::AlignCenter);
        icon->setPixmap(rowIcon.scaled(24, 24, Qt::KeepAspectRatio, Qt::SmoothTransformation));
        lay->addWidget(icon);

        auto* col = new QVBoxLayout();
        col->setSpacing(1);
        title_ = new QLabel(title, this);
        title_->setFont(theme::boldFont(title_->font()));
        col->addWidget(title_);
        sub_ = new ElidedLabel(this);
        sub_->setFullText(path);
        col->addWidget(sub_);
        lay->addLayout(col, 1);

        if (missing) {
            auto* warn = new QLabel(this);
            warn->setPixmap(theme::icon("dialog-warning", 16).pixmap(16, 16));
            warn->setToolTip(i18n("File not found"));
            lay->addWidget(warn);
            relink_ = new QToolButton(this);
            relink_->setAutoRaise(true);
            relink_->setText(i18n("Choose file…"));
            lay->addWidget(relink_);
        }
        if (remembered) {
            remember_ = new QToolButton(this);
            remember_->setAutoRaise(true);
            remember_->setToolButtonStyle(Qt::ToolButtonTextBesideIcon);
            remember_->setIconSize(QSize(14, 14));
            remember_->setIcon(theme::icon("window-close", 14));
            remember_->setText(i18n("Remembered"));
            lay->addWidget(remember_);
        }
        if (removable) {
            remove_ = new QToolButton(this);
            remove_->setAutoRaise(true);
            remove_->setIconSize(QSize(16, 16));
            remove_->setIcon(theme::icon("edit-delete", 16));
            remove_->setToolTip(i18n("Removes the entry, not the file"));
            remove_->hide();
            lay->addWidget(remove_);
        }
    }

    void setSelected(bool selected) {
        const QColor normal = palette().color(QPalette::WindowText);
        const QColor dim = palette().color(QPalette::PlaceholderText);
        const QColor highlight = palette().color(QPalette::HighlightedText);
        theme::recolor(title_, selected ? highlight : normal);
        theme::recolor(sub_, selected ? highlight : dim);
        if (remember_) theme::recolor(remember_, selected ? highlight : normal, QPalette::ButtonText);
        if (remove_) remove_->setVisible(selected);
    }

    QToolButton* removeButton() const { return remove_; }
    QToolButton* relinkButton() const { return relink_; }
    QToolButton* rememberedButton() const { return remember_; }
    bool remembered() const { return remembered_; }

private:
    QLabel* title_ = nullptr;
    ElidedLabel* sub_ = nullptr;
    QToolButton* remove_ = nullptr;
    QToolButton* relink_ = nullptr;
    QToolButton* remember_ = nullptr;
    bool remembered_ = false;
};

PickerWidget::PickerWidget(Backend* backend, QWidget* parent)
    : QWidget(parent), backend_(backend) {
    buildUi();
    reload();
}

void PickerWidget::buildUi() {
    root_ = new QVBoxLayout(this);
    root_->setContentsMargins(16, 16, 16, 16);
    root_->setSpacing(12);

    auto* head = new QHBoxLayout();
    head->setSpacing(12);
    icon_ = new QLabel(this);
    icon_->setFixedSize(48, 48);
    head->addWidget(icon_);
    auto* heading = new QVBoxLayout();
    heading->setSpacing(2);
    title_ = new QLabel(this);
    title_->setFont(theme::boldFont(title_->font(), 3));
    subtitle_ = theme::dim(new QLabel(i18n("How should the game start?"), this));
    heading->addWidget(title_);
    heading->addWidget(subtitle_);
    head->addLayout(heading, 1);

    auto* right = new QVBoxLayout();
    right->setSpacing(2);
    proton_ = theme::dim(new QLabel(this));
    right->addWidget(proton_, 0, Qt::AlignRight);
    manage_ = new QPushButton(i18n("Manage…"), this);
    manage_->setFlat(true);
    right->addWidget(manage_, 0, Qt::AlignRight);
    head->addLayout(right);
    root_->addLayout(head);

    list_ = new QListWidget(this);
    list_->setFrameShape(QFrame::NoFrame);
    list_->setSelectionMode(QAbstractItemView::SingleSelection);
    list_->setUniformItemSizes(true);
    list_->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    list_->setStyleSheet("QListWidget { background: transparent; }");
    list_->viewport()->setAutoFillBackground(false);
    list_->setAutoFillBackground(false);
    root_->addWidget(list_, 1);

    notice_ = new QLabel(this);
    notice_->setWordWrap(true);
    theme::recolor(notice_, theme::negative());
    notice_->hide();
    root_->addWidget(notice_);

    empty_ = theme::dim(new QLabel(this));
    empty_->setWordWrap(true);
    empty_->hide();
    root_->addWidget(empty_);

    auto* line = new QFrame(this);
    line->setFrameShape(QFrame::NoFrame);
    line->setFixedHeight(1);
    line->setAutoFillBackground(true);
    {
        QPalette palette = line->palette();
        palette.setColor(QPalette::Window, palette.color(QPalette::WindowText));
        line->setPalette(palette);
    }
    root_->addWidget(line);

    auto* actions = new QHBoxLayout();
    actions->setSpacing(8);
    add_ = new QToolButton(this);
    add_->setText(i18n("Add launcher"));
    add_->setIcon(theme::icon("list-add", 16));
    add_->setToolButtonStyle(Qt::ToolButtonTextBesideIcon);
    add_->setPopupMode(QToolButton::MenuButtonPopup);
    auto* addMenu = new QMenu(add_);
    addMenu->addAction(i18n("Add launcher…"), this, [this](bool) { addEntry("launchers"); });
    addMenu->addAction(i18n("Add sidecar…"), this, [this](bool) { addEntry("extras"); });
    add_->setMenu(addMenu);
    connect(add_, &QToolButton::clicked, this, [this](bool) { addEntry("launchers"); });
    actions->addWidget(add_);

    once_ = new QToolButton(this);
    once_->setText(i18n("Choose an .exe for one run…"));
    once_->setIcon(theme::icon("list-add", 16));
    once_->setToolButtonStyle(Qt::ToolButtonTextBesideIcon);
    once_->setAutoRaise(true);
    actions->addWidget(once_);

    install_ = new QToolButton(this);
    install_->setText(i18n("Install TruckersMP"));
    install_->setPopupMode(QToolButton::MenuButtonPopup);
    auto* installMenu = new QMenu(install_);
    installMenu->addAction(i18n("Install TruckersMP (ETS2)…"), this, [this](bool) { emit installRequested("truckersmp"); });
    installMenu->addAction(i18n("Install TrucksBook (ETS2/ATS)…"), this, [this](bool) { emit installRequested("trucksbook"); });
    install_->setMenu(installMenu);
    connect(install_, &QToolButton::clicked, this, [this](bool) {
        emit installRequested(appid_ == "227300" ? "truckersmp" : "trucksbook");
    });
    actions->addWidget(install_);
    actions->addStretch(1);
    root_->addLayout(actions);

    sidecars_ = new QCheckBox(i18n("Start sidecars"), this);
    sidecars_->setChecked(true);
    root_->addWidget(sidecars_);

    remember_ = new QCheckBox(i18n("Remember my choice"), this);
    root_->addWidget(remember_);

    auto* bottom = new QHBoxLayout();
    bottom->setSpacing(8);
    bottom->addStretch(1);
    cancel_ = new QPushButton(i18n("Cancel"), this);
    bottom->addWidget(cancel_);
    launch_ = new QPushButton(i18n("Launch"), this);
    launch_->setIcon(theme::icon("media-playback-start", 16));
    launch_->setDefault(true);
    bottom->addWidget(launch_);
    root_->addLayout(bottom);

    connect(list_, &QListWidget::itemSelectionChanged, this, &PickerWidget::currentChanged);
    // Double-click on a row is the same as pressing Launch: the game itself or
    // the selected launcher (a missing file still goes to the relink dialog).
    connect(list_, &QListWidget::itemDoubleClicked, this, [this](QListWidgetItem*) {
        if (launch_->isEnabled()) launch_->click();
    });
    connect(manage_, &QPushButton::clicked, this, [this](bool) { emit manageRequested(); });
    connect(cancel_, &QPushButton::clicked, this, [this](bool) { emit cancelRequested(); });
    connect(launch_, &QPushButton::clicked, this, [this](bool) {
        const int index = list_->currentRow();
        if (index < 0) return;
        if (index == 0) {
            emit launchRequested(QStringLiteral("__GAME__"), false);
            return;
        }
        const QJsonArray entries = state_.value("entries").toArray();
        if (index - 1 >= entries.size()) return;
        const QJsonObject entry = entries[index - 1].toObject();
        if (entry.value("missing").toBool()) {
            relinkEntry(entry.value("path").toString());
            return;
        }
        emit launchRequested(entry.value("path").toString(), remember_->isChecked());
    });
    connect(once_, &QToolButton::clicked, this, [this](bool) {
        const QString path = QFileDialog::getOpenFileName(
            this, i18n("Run once - choose a program"), QDir::homePath(),
            i18n("Windows programs (*.exe)"));
        if (!path.isEmpty()) emit runOnceRequested(path);
    });

    auto addShortcut = [this](const char* key, auto handler) {
        auto* shortcut = new QShortcut(QKeySequence(QString::fromUtf8(key)), this);
        connect(shortcut, &QShortcut::activated, this, handler);
    };
    addShortcut("Ctrl+N", [this] { addEntry("launchers"); });
    addShortcut("Ctrl+Shift+N", [this] { addEntry("extras"); });
    addShortcut("Ctrl+O", [this] { once_->click(); });
    addShortcut("Ctrl+,", [this] { emit manageRequested(); });
    addShortcut("Alt+Z", [this] {
        if (remember_->isEnabled()) remember_->toggle();
    });
    addShortcut("Escape", [this] { emit cancelRequested(); });
    addShortcut("Return", [this] {
        if (launch_->isEnabled()) launch_->click();
    });
    for (int i = 1; i <= 9; ++i) {
        addShortcut(QString::number(i).toUtf8().constData(), [this, i] {
            if (i <= list_->count()) list_->setCurrentRow(i - 1);
        });
    }
}

void PickerWidget::setAppId(const QString& appid) {
    appid_ = appid;
    reload();
}

void PickerWidget::reload(const QString& selectPath) {
    notice_->hide();
    state_ = backend_->picker(appid_);
    if (!Backend::ok(state_)) {
        showError(Backend::error(state_));
        return;
    }
    const QJsonObject game = state_.value("game").toObject();
    appid_ = game.value("appid").toString(appid_);
    title_->setText(game.value("name").toString(i18n("Game")));
    const QString proton = game.value("proton_name").toString();
    proton_->setText(proton.isEmpty() ? i18n("Proton: not found") : i18n("Proton: %1", proton));
    const QString iconPath = game.value("icon").toString();
    const QPixmap gameIcon(iconPath);
    icon_->setPixmap(gameIcon.isNull()
                         ? theme::icon("applications-games", 48).pixmap(48, 48)
                         : gameIcon.scaled(48, 48, Qt::KeepAspectRatio, Qt::SmoothTransformation));
    // Once both launchers are installed the installer lives in Settings only.
    const QJsonObject installers = state_.value("installers").toObject();
    install_->setVisible(!(installers.value("truckersmp").toBool()
                           && installers.value("trucksbook").toBool()));
    const QString gameName = game.value("name").toString();

    rows_.clear();
    list_->clear();
    auto* gameItem = new QListWidgetItem(list_);
    gameItem->setSizeHint(QSize(0, 54));
    auto* gameRow = new PickRow(gameName.isEmpty() ? i18n("The game itself") : gameName,
                                i18n("without a launcher"),
                                false, false, false, gameIcon, list_);
    list_->setItemWidget(gameItem, gameRow);
    rows_.push_back(gameRow);

    const QJsonArray entries = state_.value("entries").toArray();
    const QString saved = state_.value("saved").toString();
    int selectedByPath = -1;
    for (const QJsonValue& value : entries) {
        const QJsonObject entry = value.toObject();
        auto* item = new QListWidgetItem(list_);
        item->setSizeHint(QSize(0, 54));
        auto* row = new PickRow(entry.value("label").toString(),
                                entry.value("path").toString(), true,
                                entry.value("missing").toBool(),
                                entry.value("remembered").toBool() || entry.value("path").toString() == saved,
                                theme::fileIcon(entry.value("icon").toString(), 24), list_);
        list_->setItemWidget(item, row);
        rows_.push_back(row);
        if (!selectPath.isEmpty() && entry.value("path").toString() == selectPath) {
            selectedByPath = list_->count() - 1;
        }
        if (row->removeButton()) {
            const QString path = entry.value("path").toString();
            connect(row->removeButton(), &QToolButton::clicked, this, [this, path](bool) { removeEntry(path); });
        }
        if (row->relinkButton()) {
            const QString path = entry.value("path").toString();
            connect(row->relinkButton(), &QToolButton::clicked, this, [this, path](bool) { relinkEntry(path); });
        }
        if (row->rememberedButton()) {
            connect(row->rememberedButton(), &QToolButton::clicked, this, [this](bool) {
                backend_->call("forget", {{"appid", appid_}});
                reload();
            });
        }
    }

    int selected = 0;
    for (int i = 1; i < rows_.size(); ++i) {
        if (rows_[i]->remembered()) {
            selected = i;
            break;
        }
    }
    if (selected == 0 && rows_.size() > 1) selected = 1;
    if (selectedByPath >= 0) selected = selectedByPath;
    list_->setCurrentRow(selected);

    const QJsonArray extras = state_.value("extras").toArray();
    QStringList labels;
    for (const QJsonValue& value : extras) {
        labels.append(value.toObject().value("label").toString());
    }
    sidecars_->setVisible(!labels.isEmpty());
    sidecars_->setChecked(true);
    sidecars_->setText(labels.isEmpty()
                           ? i18n("Start sidecars")
                           : i18n("Start sidecars: %1", labels.join(QStringLiteral(" · "))));
    updateEmptyHint();
    update();
}

void PickerWidget::currentChanged() {
    const int row = list_->currentRow();
    for (int i = 0; i < rows_.size(); ++i) rows_[i]->setSelected(i == row);
    if (row < 0) {
        launch_->setEnabled(false);
        remember_->setEnabled(false);
        return;
    }
    if (row == 0) {
        launch_->setEnabled(true);
        remember_->setEnabled(false);
        remember_->setChecked(false);
        return;
    }
    const QJsonArray entries = state_.value("entries").toArray();
    if (row - 1 >= entries.size()) return;
    const QJsonObject entry = entries[row - 1].toObject();
    const bool missing = entry.value("missing").toBool();
    launch_->setEnabled(!missing);
    remember_->setEnabled(!missing);
    remember_->setChecked(entry.value("remembered").toBool()
                          || entry.value("path").toString() == state_.value("saved").toString());
}

bool PickerWidget::startSidecars() const {
    return !sidecars_ || sidecars_->isChecked();
}

void PickerWidget::updateEmptyHint() {
    bool hasLauncher = false;
    for (int i = 1; i < rows_.size(); ++i) {
        hasLauncher = true;
        break;
    }
    empty_->setVisible(!hasLauncher);
    if (!hasLauncher) {
        empty_->setText(i18n("You have no launchers yet. Add one with the Add launcher button "
                           "below the list, or choose an .exe for one run."));
    }
}

void PickerWidget::addEntry(const QString& which) {
    const QString path = QFileDialog::getOpenFileName(
        this, which == "extras" ? i18n("Add sidecar - choose an .exe")
                                : i18n("Add launcher - choose an .exe"),
        QDir::homePath(), i18n("Windows programs (*.exe)"));
    if (path.isEmpty()) return;
    const QString suggestion = QFileInfo(path).completeBaseName();
    bool ok = false;
    const QString label = QInputDialog::getText(
        this, which == "extras" ? i18n("Add sidecar") : i18n("Add launcher"),
        i18n("Name:"), QLineEdit::Normal, suggestion, &ok);
    if (!ok || label.trimmed().isEmpty()) return;
    const QJsonObject reply = backend_->call("list_add", {
        {"which", which},
        {"appid", appid_},
        {"label", label.trimmed()},
        {"path", path},
    });
    if (!Backend::ok(reply)) {
        showError(Backend::error(reply));
        return;
    }
    reload();
}

void PickerWidget::removeEntry(const QString& path) {
    const QJsonObject reply = backend_->call("list_remove", {
        {"which", "launchers"}, {"appid", appid_}, {"path", path},
    });
    if (!Backend::ok(reply)) {
        showError(Backend::error(reply));
        return;
    }
    reload();
}

void PickerWidget::relinkEntry(const QString& path) {
    const QString newPath = QFileDialog::getOpenFileName(
        this, i18n("Choose an .exe file"), QFileInfo(path).absolutePath(),
        i18n("Windows programs (*.exe)"));
    if (newPath.isEmpty()) return;
    const QJsonObject reply = backend_->call("list_relink", {
        {"which", "launchers"}, {"appid", appid_}, {"old", path}, {"new", newPath},
    });
    if (!Backend::ok(reply)) {
        showError(Backend::error(reply));
        return;
    }
    reload();
}

void PickerWidget::showError(const QString& message) {
    notice_->setText(message);
    notice_->show();
}

void PickerWidget::paintEvent(QPaintEvent*) {
    QPainter painter(this);
    const QJsonObject game = state_.value("game").toObject();
    theme::paintBanner(this, painter, game.value("banner").toString());
}
