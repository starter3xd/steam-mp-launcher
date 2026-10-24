#include "manager.h"

#include "i18n.h"
#include <QMessageBox>

#include "backend.h"
#include "theme.h"

#include <QAction>
#include <QButtonGroup>
#include <QCheckBox>
#include <QClipboard>
#include <QDir>
#include <QFont>
#include <QGuiApplication>
#include <QComboBox>
#include <QDesktopServices>
#include <QFileDialog>
#include <QFileInfo>
#include <QFrame>
#include <QHBoxLayout>
#include <QInputDialog>
#include <QJsonArray>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
#include <QPlainTextEdit>
#include <QPushButton>
#include <QRadioButton>
#include <QScrollArea>
#include <QScrollBar>
#include <QShortcut>
#include <QSpinBox>
#include <QStackedWidget>
#include <QTimer>
#include <QToolBar>
#include <QUrl>
#include <QVBoxLayout>

static constexpr const char* NAV[][3] = {
    {"overview", "Overview", "view-list-details"},
    {"launchers", "Launchers", "applications-games"},
    {"extras", "Sidecars", "application-x-executable"},
    {"proton", "Proton & tools", "preferences-system"},
    {"settings", "Settings", "preferences-system"},
    {"logs", "Logs", "text-x-generic"},
};

static void clearLayout(QLayout* layout) {
    if (!layout) return;
    while (QLayoutItem* item = layout->takeAt(0)) {
        if (QWidget* widget = item->widget()) widget->deleteLater();
        if (QLayout* child = item->layout()) clearLayout(child);
        delete item;
    }
}

static bool replyOk(QWidget* parent, const QJsonObject& reply) {
    if (Backend::ok(reply)) return true;
    QMessageBox::warning(parent, i18n("Operation failed"), Backend::error(reply));
    return false;
}

static QLabel* heading(const QString& text, QWidget* parent) {
    auto* label = new QLabel(text, parent);
    label->setFont(theme::boldFont(label->font(), 4));
    return label;
}

QLabel* dimLabel(const QString& text, QWidget* parent) {
    return theme::dim(new QLabel(text, parent));
}

QFrame* card(QWidget* parent) {
    auto* frame = new QFrame(parent);
    frame->setFrameShape(QFrame::StyledPanel);
    return frame;
}

class ElidedLabel : public QLabel {
public:
    explicit ElidedLabel(QWidget* parent = nullptr) : QLabel(parent) {}
    void setFullText(const QString& text) { full_ = text; setToolTip(text); updateText(); }
protected:
    void resizeEvent(QResizeEvent* event) override { QLabel::resizeEvent(event); updateText(); }
private:
    void updateText() { setText(theme::elideMiddle(fontMetrics(), full_, qMax(0, width()))); }
    QString full_;
};


Backend* ManagerPage::backend() const { return manager_->backend(); }

ManagerWidget::ManagerWidget(Backend* backend, QWidget* parent)
    : QWidget(parent), backend_(backend) {
    buildUi();
    reload();
}

void ManagerWidget::buildUi() {
    auto* root = new QVBoxLayout(this);
    root->setContentsMargins(0, 0, 0, 0);
    root->setSpacing(0);

    // A real QToolBar keeps the actions where a KDE application puts them
    // (back, refresh, per-window selector) instead of a hand-drawn header.
    auto* bar = new QToolBar(this);
    bar->setMovable(false);
    bar->setFloatable(false);
    bar->setIconSize(QSize(22, 22));
    bar->setToolButtonStyle(Qt::ToolButtonTextBesideIcon);
    bar->setContextMenuPolicy(Qt::PreventContextMenu);

    QAction* back = bar->addAction(theme::icon(QStringLiteral("go-previous"), 22),
                                   i18n("Back to the picker"));
    back->setShortcut(QKeySequence::Back);
    connect(back, &QAction::triggered, this, [this] { emit backToPicker(); });

    QAction* refresh = bar->addAction(theme::icon(QStringLiteral("view-refresh"), 22),
                                      i18n("Refresh"));
    refresh->setShortcut(QKeySequence::Refresh);
    connect(refresh, &QAction::triggered, this, &ManagerWidget::reload);

    auto* spacer = new QWidget(bar);
    spacer->setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Preferred);
    bar->addWidget(spacer);
    bar->addWidget(new QLabel(i18n("Game:"), bar));
    games_ = new QComboBox(bar);
    games_->setMinimumWidth(220);
    bar->addWidget(games_);
    root->addWidget(bar);

    // Settings-style page switcher: a flat icon list on the left, the page
    // header on the right.  Qt only (the KF6 KPageWidget was dropped so the
    // tool installs on any desktop with Qt 6).
    auto* body = new QHBoxLayout();
    body->setContentsMargins(0, 0, 0, 0);
    body->setSpacing(0);

    auto* side = new QWidget(this);
    side->setFixedWidth(220);
    auto* sideLayout = new QVBoxLayout(side);
    sideLayout->setContentsMargins(8, 8, 0, 8);
    sideLayout->setSpacing(6);
    search_ = new QLineEdit(side);
    search_->setPlaceholderText(i18n("Search…"));
    search_->setClearButtonEnabled(true);
    sideLayout->addWidget(search_);
    nav_ = new QListWidget(side);
    nav_->setFrameShape(QFrame::NoFrame);
    nav_->setIconSize(QSize(22, 22));
    nav_->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    sideLayout->addWidget(nav_, 1);
    body->addWidget(side);

    auto* line = new QFrame(this);
    line->setFrameShape(QFrame::VLine);
    body->addWidget(line);

    auto* content = new QWidget(this);
    auto* contentLayout = new QVBoxLayout(content);
    contentLayout->setContentsMargins(0, 0, 0, 0);
    contentLayout->setSpacing(0);
    pageHeader_ = new QLabel(content);
    pageHeader_->setFont(theme::boldFont(pageHeader_->font(), 2));
    pageHeader_->setContentsMargins(24, 14, 24, 8);
    contentLayout->addWidget(pageHeader_);
    stack_ = new QStackedWidget(content);
    contentLayout->addWidget(stack_, 1);
    body->addWidget(content, 1);
    root->addLayout(body, 1);

    for (const auto& item : NAV) page(QString::fromUtf8(item[0]));

    connect(nav_, &QListWidget::currentRowChanged, this, [this](int row) {
        if (loading_ || row < 0) return;
        const QString name = nav_->item(row)->data(Qt::UserRole).toString();
        pageHeader_->setText(nav_->item(row)->text());
        switchTo(name);
    });
    connect(search_, &QLineEdit::textChanged, this, [this](const QString& text) {
        for (int row = 0; row < nav_->count(); ++row) {
            QListWidgetItem* item = nav_->item(row);
            item->setHidden(!text.isEmpty() &&
                            !item->text().contains(text, Qt::CaseInsensitive));
        }
    });
    connect(games_, QOverload<int>::of(&QComboBox::currentIndexChanged), this,
            &ManagerWidget::gameChanged);
    for (int i = 0; i < static_cast<int>(sizeof(NAV) / sizeof(NAV[0])); ++i) {
        auto* shortcut = new QShortcut(QKeySequence(QStringLiteral("Ctrl+%1").arg(i + 1)), this);
        const QString name = QString::fromUtf8(NAV[i][0]);
        connect(shortcut, &QShortcut::activated, this, [this, name] { switchTo(name); });
    }
}

void ManagerWidget::setAppId(const QString& appid) {
    appid_ = appid;
    reload();
}

void ManagerWidget::reload() {
    state_ = backend_->manager(appid_);
    if (!Backend::ok(state_)) {
        return;
    }
    const QJsonObject game = state_.value("game").toObject();
    appid_ = game.value("appid").toString(appid_);
    loading_ = true;
    games_->clear();
    const QJsonArray games = state_.value("games").toArray();
    int selected = 0;
    for (int i = 0; i < games.size(); ++i) {
        const QJsonObject entry = games[i].toObject();
        games_->addItem(entry.value("name").toString(), entry.value("appid").toString());
        if (entry.value("appid").toString() == appid_) selected = i;
    }
    if (games_->count() == 0) {
        games_->addItem(game.value("name").toString(), appid_);
    } else {
        games_->setCurrentIndex(selected);
    }
    loading_ = false;

    QListWidgetItem* item = nav_ ? nav_->currentItem() : nullptr;
    QString current = item ? item->data(Qt::UserRole).toString() : QString();
    if (current.isEmpty()) current = QString::fromUtf8(NAV[0][0]);
    if (pageHeader_ && item) pageHeader_->setText(item->text());
    page(current)->load(state_);
}

void ManagerWidget::gameChanged(int index) {
    if (loading_ || index < 0) return;
    const QString appid = games_->itemData(index).toString();
    if (appid.isEmpty() || appid == appid_) return;
    appid_ = appid;
    reload();
}

// ---------------------------------------------------------------------------
// Overview
// ---------------------------------------------------------------------------

class OverviewPage : public ManagerPage {
public:
    explicit OverviewPage(ManagerWidget* manager) : ManagerPage(manager) {
        auto* outer = new QVBoxLayout(this);
        outer->setContentsMargins(24, 20, 24, 20);
        outer->setSpacing(14);
        content_ = new QVBoxLayout();
        outer->addLayout(content_);
        outer->addStretch(1);
    }

    void load(const QJsonObject& state) override {
        clearLayout(content_);
        auto* ready = card(this);
        auto* layout = new QVBoxLayout(ready);
        layout->setContentsMargins(14, 12, 14, 12);
        layout->setSpacing(8);
        layout->addWidget(new QLabel(i18n("Readiness"), ready));
        for (const QJsonValue& value : state.value("readiness").toArray()) {
            const QJsonObject check = value.toObject();
            auto* row = new QHBoxLayout();
            auto* glyph = new QLabel(check.value("state").toString() == "ok" ? "✓"
                                     : check.value("state").toString() == "error" ? "✗" : "!", ready);
            theme::recolor(glyph, check.value("state").toString() == "ok" ? theme::positive()
                                      : check.value("state").toString() == "error" ? theme::negative()
                                                                                   : theme::neutral());
            row->addWidget(glyph);
            row->addWidget(new QLabel(check.value("title").toString(), ready));
            auto* detail = new ElidedLabel(ready);
            detail->setFullText(check.value("detail").toString());
            detail->setMinimumWidth(100);
            row->addWidget(detail, 1);
            if (!check.value("view").toString().isEmpty()) {
                auto* go = new QPushButton(i18n("Go"), ready);
                go->setFlat(true);
                const QString view = check.value("view").toString();
                connect(go, &QPushButton::clicked, this, [this, view](bool) { manager_->switchTo(view); });
                row->addWidget(go);
            }
            layout->addLayout(row);
        }
        content_->addWidget(ready);

        auto* last = card(this);
        auto* lastLayout = new QHBoxLayout(last);
        lastLayout->setContentsMargins(14, 12, 14, 12);
        lastLayout->addWidget(new QLabel(i18n("Last run"), last));
        const QJsonArray logs = state.value("logs").toArray();
        if (logs.isEmpty()) {
            lastLayout->addWidget(dimLabel(i18n("No runs yet."), last));
        } else {
            const QJsonObject run = logs[0].toObject();
            auto* title = new ElidedLabel(last);
            title->setFullText(i18n("%1 · %2 · %3", run.value("time").toString(),
                                     run.value("title").toString(),
                                     run.value("status").toString()));
            lastLayout->addWidget(title, 1);
            auto* view = new QPushButton(i18n("View log"), last);
            view->setFlat(true);
            connect(view, &QPushButton::clicked, this, [this](bool) { manager_->switchTo("logs"); });
            lastLayout->addWidget(view);
        }
        content_->addWidget(last);
    }

private:
    QVBoxLayout* content_ = nullptr;
};

// ---------------------------------------------------------------------------
// Launchers / sidecars
// ---------------------------------------------------------------------------

class ListPage : public ManagerPage {
public:
    ListPage(ManagerWidget* manager, const QString& which)
        : ManagerPage(manager), which_(which) {
        auto* outer = new QVBoxLayout(this);
        outer->setContentsMargins(24, 20, 24, 20);
        outer->setSpacing(10);
        outer->addWidget(dimLabel(which == "extras"
                                      ? i18n("Programs started alongside the game (e.g. TrucksBook).")
                                      : i18n("Windows .exe files you can pick when the game starts."),
                                  this));

        auto* bar = new QHBoxLayout();
        bar->addWidget(new QLabel(i18n("List:"), this));
        auto* shared = new QRadioButton(i18n("Shared by all games"), this);
        auto* game = new QRadioButton(i18n("This game only"), this);
        auto* del = new QPushButton(theme::icon("edit-delete", 16), i18n("Delete list"), this);
        group_ = new QButtonGroup(this);
        group_->setExclusive(true);
        group_->addButton(shared);
        group_->addButton(game);
        bar->addWidget(shared);
        bar->addWidget(game);
        bar->addSpacing(16);
        bar->addWidget(del);
        bar->addStretch(1);
        auto* add = new QPushButton(theme::icon("list-add", 16), i18n("Add"), this);
        bar->addWidget(add);
        outer->addLayout(bar);

        auto* scroll = new QScrollArea(this);
        scroll->setWidgetResizable(true);
        scroll->setFrameShape(QFrame::NoFrame);
        auto* content = new QWidget(scroll);
        list_ = new QVBoxLayout(content);
        list_->setContentsMargins(0, 0, 0, 0);
        list_->setSpacing(6);
        scroll->setWidget(content);
        outer->addWidget(scroll, 1);

        connect(shared, &QPushButton::clicked, this, [this](bool) { scope("shared"); });
        connect(game, &QPushButton::clicked, this, [this](bool) { scope("game"); });
        connect(del, &QPushButton::clicked, this, [this](bool) { scope("delete"); });
        connect(add, &QPushButton::clicked, this, [this](bool) { addEntry(); });
        shared_ = shared;
        game_ = game;
    }

    void load(const QJsonObject& state) override {
        clearLayout(list_);
        const QString scopeState = state.value(which_ == "extras" ? "scope_extras" : "scope_launchers").toString();
        group_->setExclusive(false);
        // "disabled" is a per-game list kept as `<file>.off`: the shared list is
        // in effect, so the shared radio has to be checked (a 2-radio group
        // cannot show the backup state separately).
        shared_->setChecked(scopeState != "game");
        game_->setChecked(scopeState == "game");
        group_->setExclusive(true);
        const QJsonArray entries = state.value(which_ == "extras" ? "extras" : "entries").toArray();
        if (entries.isEmpty()) {
            list_->addWidget(dimLabel(which_ == "extras"
                                          ? i18n("No sidecar programs yet - add TrucksBook or another client.")
                                          : i18n("No launchers yet. Add the first one."),
                                      this));
            return;
        }
        for (const QJsonValue& value : entries) {
            const QJsonObject entry = value.toObject();
            auto* row = card(this);
            auto* layout = new QHBoxLayout(row);
            layout->setContentsMargins(12, 8, 12, 8);
            auto* glyph = new QLabel(row);
            glyph->setFixedSize(32, 32);
            glyph->setAlignment(Qt::AlignCenter);
            glyph->setPixmap(theme::fileIcon(entry.value("icon").toString(), 32));
            layout->addWidget(glyph);
            auto* text = new QVBoxLayout();
            auto* label = new QLabel(entry.value("label").toString(), row);
            label->setFont(theme::boldFont(label->font()));
            text->addWidget(label);
            auto* path = new ElidedLabel(row);
            path->setFullText(entry.value("path").toString());
            text->addWidget(path);
            layout->addLayout(text, 1);
            if (entry.value("missing").toBool()) {
                auto* missing = new QLabel(i18n("File not found"), row);
                theme::recolor(missing, theme::negative());
                layout->addWidget(missing);
                auto* relink = new QPushButton(i18n("Choose file"), row);
                layout->addWidget(relink);
                const QString oldPath = entry.value("path").toString();
                connect(relink, &QPushButton::clicked, this, [this, oldPath](bool) { relinkEntry(oldPath); });
            }
            auto* rename = new QPushButton(i18n("Rename"), row);
            layout->addWidget(rename);
            auto* remove = new QPushButton(i18n("Remove"), row);
            layout->addWidget(remove);
            const QString pathValue = entry.value("path").toString();
            const QString labelValue = entry.value("label").toString();
            connect(rename, &QPushButton::clicked, this, [this, labelValue](bool) { renameEntry(labelValue); });
            connect(remove, &QPushButton::clicked, this, [this, pathValue](bool) { removeEntry(pathValue); });
            list_->addWidget(row);
        }
        list_->addStretch(1);
    }

private:
    void addEntry() {
        const QString path = QFileDialog::getOpenFileName(
            this, i18n("Choose an .exe"), QDir::homePath(), i18n("Windows programs (*.exe)"));
        if (path.isEmpty()) return;
        bool ok = false;
        const QString label = QInputDialog::getText(
            this, i18n("Add entry"), i18n("Name:"), QLineEdit::Normal,
            QFileInfo(path).completeBaseName(), &ok);
        if (!ok || label.trimmed().isEmpty()) return;
        const QJsonObject reply = backend()->call("list_add", {
            {"which", which_}, {"appid", manager_->appId()},
            {"label", label.trimmed()}, {"path", path},
        });
        if (replyOk(this, reply)) manager_->reload();
    }

    void renameEntry(const QString& oldLabel) {
        bool ok = false;
        const QString label = QInputDialog::getText(
            this, i18n("Rename"), i18n("Name:"), QLineEdit::Normal, oldLabel, &ok);
        if (!ok || label.trimmed().isEmpty()) return;
        const QJsonObject reply = backend()->call("list_rename", {
            {"which", which_}, {"appid", manager_->appId()},
            {"old", oldLabel}, {"new", label.trimmed()},
        });
        if (replyOk(this, reply)) manager_->reload();
    }

    void relinkEntry(const QString& oldPath) {
        const QString path = QFileDialog::getOpenFileName(
            this, i18n("Choose an .exe file"), QFileInfo(oldPath).absolutePath(),
            i18n("Windows programs (*.exe)"));
        if (path.isEmpty()) return;
        const QJsonObject reply = backend()->call("list_relink", {
            {"which", which_}, {"appid", manager_->appId()}, {"old", oldPath}, {"new", path},
        });
        if (replyOk(this, reply)) manager_->reload();
    }

    void removeEntry(const QString& path) {
        const QJsonObject reply = backend()->call("list_remove", {
            {"which", which_}, {"appid", manager_->appId()}, {"path", path},
        });
        if (replyOk(this, reply)) manager_->reload();
    }

    void scope(const QString& action) {
        // Switching to the per-game list copies the shared one: otherwise the
        // picker would suddenly show no launchers at all.
        const QJsonObject reply = manager_->backend()->call("list_scope", {
            {"which", which_}, {"appid", manager_->appId()}, {"scope", action},
            {"copy", true},
        });
        if (replyOk(this, reply)) manager_->reload();
    }

    QString which_;
    QVBoxLayout* list_ = nullptr;
    QRadioButton* shared_ = nullptr;
    QRadioButton* game_ = nullptr;
    QButtonGroup* group_ = nullptr;
};

// ---------------------------------------------------------------------------
// Proton
// ---------------------------------------------------------------------------

class ProtonPage : public ManagerPage {
public:
    explicit ProtonPage(ManagerWidget* manager) : ManagerPage(manager) {
        auto* outer = new QVBoxLayout(this);
        outer->setContentsMargins(24, 20, 24, 20);
        outer->setSpacing(10);
        outer->addWidget(heading(i18n("Proton version"), this));
        auto* bar = new QHBoxLayout();
        bar->addWidget(dimLabel(i18n("The build used for the next launch."), this));
        bar->addStretch(1);
        auto* refresh = new QPushButton(i18n("Refresh"), this);
        bar->addWidget(refresh);
        outer->addLayout(bar);
        auto* scroll = new QScrollArea(this);
        scroll->setWidgetResizable(true);
        scroll->setFrameShape(QFrame::NoFrame);
        content_ = new QWidget(scroll);
        list_ = new QVBoxLayout(content_);
        list_->setContentsMargins(0, 0, 0, 0);
        scroll->setWidget(content_);
        outer->addWidget(scroll, 1);
        connect(refresh, &QPushButton::clicked, this, [this](bool) { manager_->reload(); });
    }

    void load(const QJsonObject&) override {
        clearLayout(list_);
        const QJsonObject protons = backend()->call("protons");
        const QString current = protons.value("current").toString();
        for (const QJsonValue& value : protons.value("items").toArray()) {
            const QJsonObject item = value.toObject();
            const QString dir = item.value("dir").toString();
            auto* button = new QRadioButton(item.value("name").toString(), content_);
            button->setChecked(current.startsWith(dir + "/"));
            button->setToolTip(dir);
            connect(button, &QRadioButton::toggled, this, [this, dir, button](bool checked) {
                if (!checked) return;
                const QJsonObject reply = backend()->call("set_proton", {{"path", dir}});
                if (replyOk(this, reply)) {
                    QTimer::singleShot(0, this, [this] { manager_->reload(); });
                }
            });
            list_->addWidget(button);
            auto* source = dimLabel(item.value("source").toString(), content_);
            list_->addWidget(source);
        }
        if (protons.value("items").toArray().isEmpty()) {
            list_->addWidget(dimLabel(i18n("No Proton found. Install Proton in Steam or add a "
                                         "build to compatibilitytools.d."),
                                      content_));
        }
        list_->addStretch(1);
    }

private:
    QWidget* content_ = nullptr;
    QVBoxLayout* list_ = nullptr;
};

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

class SettingsPage : public ManagerPage {
public:
    explicit SettingsPage(ManagerWidget* manager) : ManagerPage(manager) {
        auto* outer = new QVBoxLayout(this);
        outer->setContentsMargins(0, 0, 0, 0);
        scroll_ = new QScrollArea(this);
        scroll_->setWidgetResizable(true);
        scroll_->setFrameShape(QFrame::NoFrame);
        content_ = new QWidget(scroll_);
        list_ = new QVBoxLayout(content_);
        list_->setContentsMargins(24, 20, 24, 20);
        list_->setSpacing(14);
        scroll_->setWidget(content_);
        outer->addWidget(scroll_);
        status_ = new QLabel(this);
        status_->hide();
        outer->addWidget(status_);
    }

    void load(const QJsonObject&) override {
        clearLayout(list_);
        // The installers were moved out of the picker once both are present;
        // Settings keeps them reachable.
        auto* installers = card(content_);
        auto* installLayout = new QVBoxLayout(installers);
        installLayout->setContentsMargins(14, 12, 14, 12);
        installLayout->setSpacing(8);
        auto* installHead = new QHBoxLayout();
        installHead->addWidget(new QLabel(i18n("Installers"), installers));
        installHead->addStretch(1);
        installLayout->addLayout(installHead);
        installLayout->addWidget(dimLabel(
            i18n("Install or repair the launchers; the picker button hides once "
                 "TruckersMP and TrucksBook are both present."),
            installers));
        auto* installRow = new QHBoxLayout();
        auto* tmpButton = new QPushButton(i18n("Install TruckersMP…"), installers);
        auto* tbButton = new QPushButton(i18n("Install TrucksBook…"), installers);
        connect(tmpButton, &QPushButton::clicked, this,
                [this](bool) { manager_->requestInstall("truckersmp"); });
        connect(tbButton, &QPushButton::clicked, this,
                [this](bool) { manager_->requestInstall("trucksbook"); });
        installRow->addWidget(tmpButton);
        installRow->addWidget(tbButton);
        installRow->addStretch(1);
        installLayout->addLayout(installRow);
        list_->addWidget(installers);

        const QJsonObject reply = backend()->call("settings");
        const QJsonObject current = reply.value("current").toObject();
        QHash<QString, QJsonObject> specs;
        for (const QJsonValue& value : reply.value("specs").toArray()) {
            const QJsonObject spec = value.toObject();
            specs.insert(spec.value("key").toString(), spec);
        }
        for (const QJsonValue& value : reply.value("groups").toArray()) {
            const QJsonObject group = value.toObject();
            auto* frame = card(content_);
            auto* layout = new QVBoxLayout(frame);
            layout->setContentsMargins(14, 12, 14, 12);
            layout->setSpacing(8);
            auto* head = new QHBoxLayout();
            head->addWidget(new QLabel(group.value("title").toString(), frame));
            head->addStretch(1);
            auto* restore = new QPushButton(i18n("Restore defaults"), frame);
            restore->setFlat(true);
            head->addWidget(restore);
            layout->addLayout(head);
            const QString groupName = group.value("name").toString();
            connect(restore, &QPushButton::clicked, this, [this, groupName](bool) {
                backend()->call("restore_settings", {{"group", groupName}});
                manager_->reload();
            });
            for (const QJsonValue& keyValue : group.value("keys").toArray()) {
                const QString key = keyValue.toString();
                const QJsonObject spec = specs.value(key);
                if (spec.isEmpty()) continue;
                auto* row = new QHBoxLayout();
                auto* name = new QLabel(spec.value("title").toString(), frame);
                name->setMinimumWidth(240);
                row->addWidget(name);
                const QString kind = spec.value("kind").toString();
                if (kind == "int") {
                    auto* spin = new QSpinBox(frame);
                    spin->setRange(spec.value("minimum").toInt(), spec.value("maximum").toInt());
                    spin->setSpecialValueText(spec.value("special_zero").toString());
                    spin->setValue(current.value(key).toString().toInt());
                    connect(spin, QOverload<int>::of(&QSpinBox::valueChanged), this, [this, key](int value) {
                        backend()->call("set_setting", {{"key", key}, {"value", QString::number(value)}});
                        flashSaved();
                    });
                    row->addWidget(spin);
                } else {
                    auto* combo = new QComboBox(frame);
                    const QJsonArray choices = spec.value("choices").toArray();
                    if (choices.isEmpty() && key == "UI_LANGUAGE") {
                        combo->addItem(i18n("English (default)"), "");
                        combo->addItem(i18n("System language"), "auto");
                    } else {
                        for (const QJsonValue& choiceValue : choices) {
                            const QJsonObject choice = choiceValue.toObject();
                            combo->addItem(choice.value("label").toString(), choice.value("value").toString());
                        }
                    }
                    const QString value = current.value(key).toString();
                    const int index = combo->findData(value);
                    if (index >= 0) combo->setCurrentIndex(index);
                    connect(combo, QOverload<int>::of(&QComboBox::currentIndexChanged), this,
                            [this, key, combo](int) {
                                backend()->call("set_setting", {{"key", key}, {"value", combo->currentData().toString()}});
                                flashSaved();
                            });
                    row->addWidget(combo);
                }
                row->addStretch(1);
                auto* keyLabel = dimLabel(key, frame);
                row->addWidget(keyLabel);
                layout->addLayout(row);
            }
            list_->addWidget(frame);
        }
        list_->addWidget(dimLabel(i18n("Settings are saved immediately."), content_));
        list_->addStretch(1);
        scroll_->verticalScrollBar()->setValue(0);
    }

private:
    void flashSaved() {
        // A small, non-modal confirmation; the write already happened.
        status_->setText(i18n("Saved ✓"));
        status_->setVisible(true);
        QTimer::singleShot(1500, status_, &QLabel::hide);
    }

    QScrollArea* scroll_ = nullptr;
    QWidget* content_ = nullptr;
    QVBoxLayout* list_ = nullptr;
    QLabel* status_ = nullptr;
};

// ---------------------------------------------------------------------------
// Logs
// ---------------------------------------------------------------------------

class LogsPage : public ManagerPage {
public:
    explicit LogsPage(ManagerWidget* manager) : ManagerPage(manager) {
        auto* outer = new QVBoxLayout(this);
        outer->setContentsMargins(24, 20, 24, 20);
        outer->setSpacing(10);
        auto* top = new QHBoxLayout();
        top->addWidget(new QLabel(i18n("Run:"), this));
        combo_ = new QComboBox(this);
        top->addWidget(combo_, 1);
        auto* folder = new QPushButton(i18n("Open folder"), this);
        top->addWidget(folder);
        outer->addLayout(top);

        auto* tools = new QHBoxLayout();
        search_ = new QLineEdit(this);
        search_->setPlaceholderText(i18n("Search…"));
        search_->setClearButtonEnabled(true);
        tools->addWidget(search_, 1);
        filter_ = new QComboBox(this);
        filter_->addItem(i18n("Everything"), "all");
        filter_->addItem(i18n("Errors and warnings"), "problems");
        tools->addWidget(filter_);
        follow_ = new QCheckBox(i18n("Follow live"), this);
        tools->addWidget(follow_);
        auto* copy = new QPushButton(i18n("Copy"), this);
        tools->addWidget(copy);
        auto* clear = new QPushButton(i18n("Clear old logs…"), this);
        tools->addWidget(clear);
        outer->addLayout(tools);

        viewer_ = new QPlainTextEdit(this);
        viewer_->setReadOnly(true);
        viewer_->setFont(QFont("monospace"));
        outer->addWidget(viewer_, 1);

        timer_ = new QTimer(this);
        timer_->setInterval(1000);
        connect(timer_, &QTimer::timeout, this, &LogsPage::tick);
        connect(combo_, QOverload<int>::of(&QComboBox::currentIndexChanged), this, [this](int) { loadCurrent(); });
        connect(search_, &QLineEdit::textChanged, this, [this](const QString&) { render(); });
        connect(filter_, QOverload<int>::of(&QComboBox::currentIndexChanged), this, [this](int) { render(); });
        connect(follow_, &QCheckBox::toggled, this, [this](bool on) { if (on) timer_->start(); else timer_->stop(); });
        connect(copy, &QPushButton::clicked, this, [this](bool) { QGuiApplication::clipboard()->setText(viewer_->toPlainText()); });
        connect(folder, &QPushButton::clicked, this, [this](bool) {
            QDesktopServices::openUrl(QUrl::fromLocalFile(manager_->state().value("logs_dir").toString()));
        });
        connect(clear, &QPushButton::clicked, this, [this](bool) {
            bool ok = false;
            const int days = QInputDialog::getInt(this, i18n("Delete old logs"), i18n("Older than days:"), 7, 1, 3650, 1, &ok);
            if (ok) backend()->call("clear_logs", {{"days", days}});
        });
    }

    void load(const QJsonObject& state) override {
        timer_->stop();
        combo_->blockSignals(true);
        combo_->clear();
        for (const QJsonValue& value : state.value("logs").toArray()) {
            const QJsonObject run = value.toObject();
            combo_->addItem(QStringLiteral("%1 · %2 · %3").arg(run.value("time").toString(),
                                                               run.value("title").toString(),
                                                               run.value("status").toString()),
                            run.value("path").toString());
        }
        combo_->blockSignals(false);
        loadCurrent();
        // `load` stops the follow timer; without this the checkbox stays
        // checked while nothing follows anymore.
        if (follow_->isChecked()) timer_->start();
    }

private:
    void loadCurrent() {
        raw_.clear();
        const QString path = combo_->currentData().toString();
        if (!path.isEmpty()) {
            const QJsonObject reply = backend()->call("read_log", {{"path", path}});
            raw_ = reply.value("text").toString();
        }
        render();
    }

    void render() {
        const QString query = search_->text().toLower();
        const bool problems = filter_->currentData().toString() == "problems";
        QStringList lines;
        for (const QString& line : raw_.split('\n')) {
            if (!query.isEmpty() && !line.toLower().contains(query)) continue;
            if (problems && !line.contains("ERROR") && !line.contains("skipped")
                && !line.contains("missing") && !line.contains("warning")) {
                continue;
            }
            lines.append(line);
        }
        viewer_->setPlainText(lines.join('\n'));
    }

    void tick() {
        if (!follow_->isChecked()) return;
        const QString path = combo_->currentData().toString();
        if (path.isEmpty()) return;
        const QJsonObject reply = backend()->call("read_log", {{"path", path}});
        const QString text = reply.value("text").toString();
        if (text != raw_) {
            raw_ = text;
            render();
        }
    }

    QComboBox* combo_ = nullptr;
    QLineEdit* search_ = nullptr;
    QComboBox* filter_ = nullptr;
    QCheckBox* follow_ = nullptr;
    QPlainTextEdit* viewer_ = nullptr;
    QTimer* timer_ = nullptr;
    QString raw_;
};

void ManagerWidget::requestInstall(const QString& kind) {
    emit installRequested(kind);
}

ManagerPage* ManagerWidget::page(const QString& name) {
    if (ManagerPage* existing = pages_.value(name)) return existing;
    ManagerPage* created = nullptr;
    if (name == "overview") created = new OverviewPage(this);
    else if (name == "launchers") created = new ListPage(this, "launchers");
    else if (name == "extras") created = new ListPage(this, "extras");
    else if (name == "proton") created = new ProtonPage(this);
    else if (name == "settings") created = new SettingsPage(this);
    else created = new LogsPage(this);
    pages_.insert(name, created);

    QString title;
    QString iconName;
    for (const auto& entry : NAV) {
        if (name == QLatin1String(entry[0])) {
            title = i18n(entry[1]);
            iconName = QString::fromUtf8(entry[2]);
            break;
        }
    }
    auto* row = new QListWidgetItem(theme::icon(iconName, 22),
                                    title.isEmpty() ? name : title, nav_);
    row->setData(Qt::UserRole, name);
    stack_->addWidget(created);
    if (nav_->count() == 1) nav_->setCurrentRow(0);
    return created;
}

void ManagerWidget::switchTo(const QString& name) {
    ManagerPage* target = page(name);
    for (int row = 0; row < nav_->count(); ++row) {
        if (nav_->item(row)->data(Qt::UserRole).toString() != name) continue;
        loading_ = true;
        nav_->setCurrentRow(row);
        loading_ = false;
        pageHeader_->setText(nav_->item(row)->text());
        break;
    }
    stack_->setCurrentWidget(target);
    if (!state_.isEmpty()) target->load(state_);
}

