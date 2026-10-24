#pragma once

#include <QHash>
#include <QJsonObject>
#include <QWidget>

class Backend;
class ManagerPage;
class QComboBox;
class QLabel;
class QLineEdit;
class QListWidget;
class QStackedWidget;

class ManagerWidget : public QWidget {
    Q_OBJECT
public:
    explicit ManagerWidget(Backend* backend, QWidget* parent = nullptr);

    Backend* backend() const { return backend_; }
    const QJsonObject& state() const { return state_; }
    QString appId() const { return appid_; }
    void setAppId(const QString& appid);
    void reload();
    void switchTo(const QString& name);
    /// Settings page -> AppWindow: open the TruckersMP/TrucksBook wizard.
    void requestInstall(const QString& kind);

signals:
    void backToPicker();
    void installRequested(const QString& kind);

private:
    ManagerPage* page(const QString& name);
    void buildUi();
    void gameChanged(int index);

    Backend* backend_ = nullptr;
    QJsonObject state_;
    QString appid_;
    QComboBox* games_ = nullptr;
    QLabel* pageHeader_ = nullptr;
    QLineEdit* search_ = nullptr;
    QListWidget* nav_ = nullptr;
    QStackedWidget* stack_ = nullptr;
    QHash<QString, ManagerPage*> pages_;
    bool loading_ = false;
};

class ManagerPage : public QWidget {
public:
    explicit ManagerPage(ManagerWidget* manager) : QWidget(manager), manager_(manager) {}
    virtual void load(const QJsonObject& state) = 0;

protected:
    Backend* backend() const;
    ManagerWidget* manager_;
};
