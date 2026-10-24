#pragma once

#include <QDialog>

class QLabel;
class QProgressBar;
class QPushButton;
class QTimer;

/// The countdown shown before a remembered choice starts.
/// `exec()` returns 0 = launch, 1 = cancel, 2 = change choice.
class ConfirmWindow : public QDialog {
public:
    enum Decision { Launch = 0, Cancel = 1, Change = 2 };
    ConfirmWindow(const QString& label, const QString& game, int seconds,
                  QWidget* parent = nullptr);

protected:
    void reject() override;

private:
    void tick();
    void choose(int decision);

    int seconds_ = 3;
    int remaining_ = 3;
    QLabel* count_ = nullptr;
    QProgressBar* bar_ = nullptr;
    QTimer* timer_ = nullptr;
    QPushButton* run_ = nullptr;
};
