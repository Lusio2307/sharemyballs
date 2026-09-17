use std::sync::{Arc, Mutex};

use futures_util::SinkExt;
use iced::widget::row;
use iced::Alignment::Center;
use iced::{executor, Application, Command, Length, Subscription};
use tokio::sync::mpsc::{channel, Receiver, Sender};

use crate::capture::capturer::Capturer;
use crate::column_iced;
use crate::gui::component::sharing::SharingPage;
use crate::gui::component::start::StartPage;
use crate::gui::component::{sharing, start, Component};
use crate::gui::theme::widget::Element;
use crate::gui::theme::Theme;

pub struct App {
    /// Shared with the embedded server's admin control plane, which is why it is
    /// no longer owned outright. `std::sync::Mutex` because the GUI only uses
    /// synchronous `Capturer` methods on it.
    capturer: Arc<Mutex<Capturer>>,
    pub start_page: StartPage,
    pub sharing_page: SharingPage,
    intermediate_update_receiver: Option<Receiver<()>>,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum Message {
    Start(start::Message),
    Sharing(sharing::Message),
    Ignore,
    UpdateChannel(Sender<()>),
}

impl Application for App {
    type Executor = executor::Default;
    type Message = Message;

    type Theme = Theme;
    /// `Args` and `Config` now ride along inside the `Capturer` (both are public
    /// fields), so the flags only carry the shared handle and the update channel.
    type Flags = (Arc<Mutex<Capturer>>, Receiver<()>);

    fn new((capturer, intermediate_update_receiver): Self::Flags) -> (Self, Command<Message>) {
        // Unattended start: skip the "Start Sharing" click. Together with
        // `auto_accept` this means the app needs no interaction after launch,
        // which is what makes it viable to start from a logon task.
        let auto_start = capturer.lock().unwrap().config.auto_start;

        if auto_start {
            info!("auto_start is enabled; beginning to share immediately");
            capturer.lock().unwrap().run();
        }

        (
            App {
                capturer,
                start_page: StartPage {},
                sharing_page: SharingPage::new(),
                intermediate_update_receiver: Some(intermediate_update_receiver),
            },
            Command::none(),
        )
    }

    fn title(&self) -> String {
        String::from("Mira Sharer")
    }

    fn update(&mut self, message: Message) -> Command<Message> {
        match message {
            Message::Start(message) => {
                let mut capturer = self.capturer.lock().unwrap();
                self.start_page.update(
                    message,
                    start::UpdateProps {
                        capturer: &mut capturer,
                    },
                )
            }
            Message::Sharing(message) => {
                let mut capturer = self.capturer.lock().unwrap();
                let viewer_manager = capturer.get_viewer_manager();
                self.sharing_page.update(
                    message,
                    sharing::UpdateProps {
                        capturer: &mut capturer,
                        viewer_manager,
                    },
                )
            }
            Message::Ignore => Command::none(),
            Message::UpdateChannel(channel) => {
                let mut receiver = self.intermediate_update_receiver.take().unwrap();
                tokio::spawn(async move {
                    while let Some(_) = receiver.recv().await {
                        channel.send(()).await.unwrap();
                    }
                });
                Command::none()
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let capturer = self.capturer.lock().unwrap();
        let is_sharing = capturer.is_running();
        let element: Element<Message> = row![column_iced![if is_sharing {
            let viewer_manager = capturer.get_viewer_manager();
            let handle = tokio::runtime::Handle::current();
            let (pending_viewers, viewing_viewers) = tokio::task::block_in_place(move || {
                handle.block_on(async move {
                    let pending_viewers = viewer_manager.get_pending_viewers().await;
                    let viewing_viewers = viewer_manager.get_viewing_viewers().await;
                    (pending_viewers, viewing_viewers)
                })
            });

            self.sharing_page.view(sharing::ViewProps {
                room_id: capturer.get_room_id().unwrap_or_default(),
                room_password: capturer.get_room_password().unwrap_or_default(),
                invite_link: capturer.get_invite_link().unwrap_or_default(),
                pending_viewers,
                viewing_viewers,
            })
        } else {
            self.start_page.view(start::ViewProps {
                capturer: &capturer,
            })
        }]
        .spacing(12)]
        .align_items(Center)
        .height(Length::Fill)
        .into();
        element
        // element.explain(iced::Color::WHITE)
    }

    fn theme(&self) -> Self::Theme {
        Theme::Dark
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        iced::subscription::channel("updates", 10, |mut s| async move {
            let (sender, mut receiver) = channel(10);
            s.send(Message::UpdateChannel(sender)).await.unwrap();
            loop {
                let _ = receiver.recv().await;
                s.send(Message::Ignore).await.unwrap();
            }
        })
    }
}
