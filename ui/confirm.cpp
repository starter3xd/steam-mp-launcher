#include "confirm.h"
#include "i18n.h"

#include "theme.h"

#include <QHBoxLayout>
#include <QLabel>
#include <QProgressBar>
#include <QPushButton>
#include <QTimer>
#include <QVBoxLayout>

ConfirmWindow::ConfirmWindow(const QString& label, const QString& game, int seconds,
                             QWidget* parent)
    : QDialog(parent), seconds_(qMax(1, seconds)), remaining_(qMax(1, seconds)) {
    setWindowTitle(i18n("%1 · Steam MP Launcher", game));
    setWindowIcon(theme::appIcon());
    setMinimumWidth(420);

    auto* root = new QVBoxLayout(this);
    root->setContentsMargins(20, 18, 20, 16);
    root->setSpacing(6);

    auto* head = new QLabel(i18n("Starting soon"), this);
    head->setFont(theme::boldFont(head->font(), 1));
    root->addWidget(head);

    auto* name = new QLabel(label.isEmpty() ? i18n("(remembered launcher)") : label, this);
    name->setFont(theme::boldFont(name->font(), 4));
    name->setWordWrap(true);
    root->addWidget(name);
    if (!game.isEmpty()) root->addWidget(theme::dim(new QLabel(i18n("Game: %1", game), this)));

    root->addSpacing(8);
    count_ = new QLabel(QString::number(remaining_), this);
    count_->setAlignment(Qt::AlignCenter);
    auto countFont = count_->font();
    countFont.setPointSizeF(countFont.pointSizeF() + 14);
    countFont.setBold(true);
    count_->setFont(countFont);
    root->addWidget(count_);

    bar_ = new QProgressBar(this);
    bar_->setRange(0, seconds_);
    bar_->setValue(remaining_);
    bar_->setTextVisible(false);
    bar_->setFixedHeight(6);
    root->addWidget(bar_);

    auto* hint = theme::dim(new QLabel(i18n("Enter - start now · Esc - cancel"), this));
    hint->setAlignment(Qt::AlignCenter);
    root->addWidget(hint);

    root->addSpacing(6);
    auto* buttons = new QHBoxLayout();
    buttons->setSpacing(8);
    auto* change = new QPushButton(i18n("Change choice"), this);
    change->setToolTip(i18n("Return to the full launcher picker"));
    buttons->addWidget(change);
    buttons->addStretch(1);
    auto* cancel = new QPushButton(i18n("Cancel"), this);
    buttons->addWidget(cancel);
    run_ = new QPushButton(i18n("Start now"), this);
    run_->setDefault(true);
    buttons->addWidget(run_);
    root->addLayout(buttons);

    connect(change, &QPushButton::clicked, this, [this](bool) { choose(Change); });
    connect(cancel, &QPushButton::clicked, this, [this](bool) { choose(Cancel); });
    connect(run_, &QPushButton::clicked, this, [this](bool) { choose(Launch); });

    timer_ = new QTimer(this);
    timer_->setInterval(1000);
    connect(timer_, &QTimer::timeout, this, &ConfirmWindow::tick);
    timer_->start();
    run_->setFocus();
}

void ConfirmWindow::tick() {
    --remaining_;
    if (remaining_ <= 0) {
        choose(Launch);
        return;
    }
    count_->setText(QString::number(remaining_));
    bar_->setValue(remaining_);
}

void ConfirmWindow::choose(int decision) {
    if (timer_) timer_->stop();
    done(decision);
}

void ConfirmWindow::reject() {
    if (timer_) timer_->stop();
    done(Cancel);
}
