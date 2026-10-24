#pragma once

#include <QJsonObject>
#include <QVector>
#include <QWidget>

class Backend;
class QCheckBox;
class QLabel;
class QListWidget;
class QPushButton;
class QToolButton;
class QVBoxLayout;
class PickRow;

class PickerWidget : public QWidget {
    Q_OBJECT
public:
    explicit PickerWidget(Backend* backend, QWidget* parent = nullptr);

    void setAppId(const QString& appid);
    QString appId() const { return appid_; }
    void reload(const QString& selectPath = {});
    /// The picker's "start sidecars" switch (on unless the user cleared it).
    bool startSidecars() const;

signals:
    void manageRequested();
    void launchRequested(const QString& exe, bool remember);
    void runOnceRequested(const QString& exe);
    void installRequested(const QString& kind);
    void cancelRequested();

protected:
    void paintEvent(QPaintEvent* event) override;

private:
    void buildUi();
    void addEntry(const QString& which);
    void removeEntry(const QString& path);
    void relinkEntry(const QString& path);
    void currentChanged();
    void updateEmptyHint();
    void showError(const QString& message);

    Backend* backend_ = nullptr;
    QJsonObject state_;
    QString appid_;

    QVBoxLayout* root_ = nullptr;
    QLabel* icon_ = nullptr;
    QLabel* title_ = nullptr;
    QLabel* subtitle_ = nullptr;
    QLabel* proton_ = nullptr;
    QLabel* notice_ = nullptr;
    QLabel* empty_ = nullptr;
    QCheckBox* sidecars_ = nullptr;
    QListWidget* list_ = nullptr;
    QCheckBox* remember_ = nullptr;
    QPushButton* manage_ = nullptr;
    QPushButton* cancel_ = nullptr;
    QPushButton* launch_ = nullptr;
    QToolButton* add_ = nullptr;
    QToolButton* once_ = nullptr;
    QToolButton* install_ = nullptr;
    QVector<PickRow*> rows_;
};
