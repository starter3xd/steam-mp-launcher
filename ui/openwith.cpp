#include "openwith.h"
#include "i18n.h"

#include "backend.h"
#include "theme.h"

#include <QComboBox>
#include <QJsonArray>
#include <QFileInfo>
#include <QHBoxLayout>
#include <QInputDialog>
#include <QLabel>
#include <QLineEdit>
#include <QPushButton>
#include <QVBoxLayout>

OpenWithDialog::OpenWithDialog(Backend* backend, const QString& file, QWidget* parent)
    : QDialog(parent), backend_(backend), file_(file) {
    setWindowTitle(i18n("Open with · Steam MP Launcher"));
    resize(500, 240);
    state_ = backend_->picker();
    appid_ = state_.value("game").toObject().value("appid").toString();

    auto* root = new QVBoxLayout(this);
    root->setContentsMargins(20, 18, 20, 16);
    root->setSpacing(8);
    auto* head = new QLabel(i18n("What should happen with this program?"), this);
    head->setFont(theme::boldFont(head->font(), 2));
    root->addWidget(head);
    const QJsonObject iconReply = backend_->call("icon", {{"path", file_}});
    auto* fileRow = new QHBoxLayout();
    fileRow->setSpacing(10);
    auto* icon = new QLabel(this);
    icon->setFixedSize(32, 32);
    icon->setPixmap(theme::fileIcon(iconReply.value("icon").toString(), 32));
    fileRow->addWidget(icon);
    auto* fileText = new QVBoxLayout();
    fileText->setSpacing(1);
    auto* name = new QLabel(QFileInfo(file_).fileName(), this);
    name->setFont(theme::boldFont(name->font(), 1));
    fileText->addWidget(name);
    auto* path = theme::dim(new QLabel(file_, this));
    path->setWordWrap(true);
    fileText->addWidget(path);
    fileRow->addLayout(fileText, 1);
    root->addLayout(fileRow);
    const QJsonObject gamesReply = backend_->call("games");
    const QJsonArray games = gamesReply.value("items").toArray();
    if (games.size() > 1) {
        auto* row = new QHBoxLayout();
        row->addWidget(new QLabel(i18n("Game:"), this));
        auto* combo = new QComboBox(this);
        for (const QJsonValue& value : games) {
            const QJsonObject game = value.toObject();
            combo->addItem(game.value("name").toString(), game.value("appid").toString());
        }
        const int index = combo->findData(appid_);
        if (index >= 0) combo->setCurrentIndex(index);
        connect(combo, QOverload<int>::of(&QComboBox::currentIndexChanged), this, [this, combo](int) {
            appid_ = combo->currentData().toString();
            state_ = backend_->call("picker", {{"appid", appid_}});
        });
        row->addWidget(combo, 1);
        root->addLayout(row);
    }
    root->addWidget(theme::dim(new QLabel(
        i18n("It will be added to the list of the selected game."), this)));
    root->addStretch(1);

    status_ = new QLabel(this);
    theme::recolor(status_, theme::negative());
    status_->setWordWrap(true);
    status_->hide();
    root->addWidget(status_);

    auto* buttons = new QHBoxLayout();
    buttons->setSpacing(8);
    auto* launcher = new QPushButton(i18n("Add launcher"), this);
    auto* sidecar = new QPushButton(i18n("Add sidecar"), this);
    auto* once = new QPushButton(i18n("Run once"), this);
    once->setDefault(true);
    auto* cancel = new QPushButton(i18n("Cancel"), this);
    buttons->addWidget(launcher);
    buttons->addWidget(sidecar);
    buttons->addStretch(1);
    buttons->addWidget(once);
    buttons->addWidget(cancel);
    root->addLayout(buttons);

    connect(launcher, &QPushButton::clicked, this, [this](bool) { addTo("launchers"); });
    connect(sidecar, &QPushButton::clicked, this, [this](bool) { addTo("extras"); });
    connect(once, &QPushButton::clicked, this, [this](bool) {
        runOnce_ = true;
        accept();
    });
    connect(cancel, &QPushButton::clicked, this, [this](bool) { reject(); });
}

QString OpenWithDialog::labelFor(const QString& path) const {
    return QFileInfo(path).completeBaseName();
}

void OpenWithDialog::addTo(const QString& which) {
    bool ok = false;
    const QString label = QInputDialog::getText(
        this, which == "extras" ? i18n("Add sidecar") : i18n("Add launcher"),
        i18n("Name:"), QLineEdit::Normal, labelFor(file_), &ok);
    if (!ok || label.trimmed().isEmpty()) return;
    const QJsonObject reply = backend_->call("list_add", {
        {"which", which},
        {"appid", appid_},
        {"label", label.trimmed()},
        {"path", file_},
    });
    if (!Backend::ok(reply)) {
        status_->setText(Backend::error(reply));
        status_->show();
        return;
    }
    accept();
}
