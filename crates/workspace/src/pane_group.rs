use crate::{FollowerStatesByLeader, JoinProject, Pane, Workspace};
use anyhow::{anyhow, Result};
use call::{ActiveCall, ParticipantLocation};
use gpui::{
    elements::*,
    geometry::{rect::RectF, vector::Vector2F},
    Axis, Border, CursorStyle, ModelHandle, MouseButton, RenderContext, ViewHandle,
};
use project::Project;
use serde::Deserialize;
use theme::Theme;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaneGroup {
    root: Member,
}

impl PaneGroup {
    pub fn new(pane: ViewHandle<Pane>) -> Self {
        Self {
            root: Member::Pane(pane),
        }
    }

    pub fn split(
        &mut self,
        old_pane: &ViewHandle<Pane>,
        new_pane: &ViewHandle<Pane>,
        direction: SplitDirection,
    ) -> Result<()> {
        match &mut self.root {
            Member::Pane(pane) => {
                if pane == old_pane {
                    self.root = Member::new_axis(old_pane.clone(), new_pane.clone(), direction);
                    Ok(())
                } else {
                    Err(anyhow!("Pane not found"))
                }
            }
            Member::Axis(axis) => axis.split(old_pane, new_pane, direction),
        }
    }

    /// Returns:
    /// - Ok(true) if it found and removed a pane
    /// - Ok(false) if it found but did not remove the pane
    /// - Err(_) if it did not find the pane
    pub fn remove(&mut self, pane: &ViewHandle<Pane>) -> Result<bool> {
        match &mut self.root {
            Member::Pane(_) => Ok(false),
            Member::Axis(axis) => {
                if let Some(last_pane) = axis.remove(pane)? {
                    self.root = last_pane;
                }
                Ok(true)
            }
        }
    }

    pub(crate) fn render(
        &self,
        project: &ModelHandle<Project>,
        theme: &Theme,
        follower_states: &FollowerStatesByLeader,
        active_call: Option<&ModelHandle<ActiveCall>>,
        cx: &mut RenderContext<Workspace>,
    ) -> ElementBox {
        self.root
            .render(project, theme, follower_states, active_call, cx)
    }

    pub(crate) fn panes(&self) -> Vec<&ViewHandle<Pane>> {
        let mut panes = Vec::new();
        self.root.collect_panes(&mut panes);
        panes
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Member {
    Axis(PaneAxis),
    Pane(ViewHandle<Pane>),
}

impl Member {
    fn new_axis(
        old_pane: ViewHandle<Pane>,
        new_pane: ViewHandle<Pane>,
        direction: SplitDirection,
    ) -> Self {
        use Axis::*;
        use SplitDirection::*;

        let axis = match direction {
            Up | Down => Vertical,
            Left | Right => Horizontal,
        };

        let members = match direction {
            Up | Left => vec![Member::Pane(new_pane), Member::Pane(old_pane)],
            Down | Right => vec![Member::Pane(old_pane), Member::Pane(new_pane)],
        };

        Member::Axis(PaneAxis { axis, members })
    }

    pub fn render(
        &self,
        project: &ModelHandle<Project>,
        theme: &Theme,
        follower_states: &FollowerStatesByLeader,
        active_call: Option<&ModelHandle<ActiveCall>>,
        cx: &mut RenderContext<Workspace>,
    ) -> ElementBox {
        enum FollowIntoExternalProject {}

        match self {
            Member::Pane(pane) => {
                let leader = follower_states
                    .iter()
                    .find_map(|(leader_id, follower_states)| {
                        if follower_states.contains_key(pane) {
                            Some(leader_id)
                        } else {
                            None
                        }
                    })
                    .and_then(|leader_id| {
                        let room = active_call?.read(cx).room()?.read(cx);
                        let collaborator = project.read(cx).collaborators().get(leader_id)?;
                        let participant = room.remote_participants().get(&leader_id)?;
                        Some((collaborator.replica_id, participant))
                    });

                let border = if let Some((replica_id, _)) = leader.as_ref() {
                    let leader_color = theme.editor.replica_selection_style(*replica_id).cursor;
                    let mut border = Border::all(theme.workspace.leader_border_width, leader_color);
                    border
                        .color
                        .fade_out(1. - theme.workspace.leader_border_opacity);
                    border.overlay = true;
                    border
                } else {
                    Border::default()
                };

                let prompt = if let Some((_, leader)) = leader {
                    match leader.location {
                        ParticipantLocation::SharedProject {
                            project_id: leader_project_id,
                        } => {
                            if Some(leader_project_id) == project.read(cx).remote_id() {
                                None
                            } else {
                                let leader_user = leader.user.clone();
                                let leader_user_id = leader.user.id;
                                Some(
                                    MouseEventHandler::<FollowIntoExternalProject>::new(
                                        pane.id(),
                                        cx,
                                        |_, _| {
                                            Label::new(
                                                format!(
                                                    "Follow {} on their active project",
                                                    leader_user.github_login,
                                                ),
                                                theme
                                                    .workspace
                                                    .external_location_message
                                                    .text
                                                    .clone(),
                                            )
                                            .contained()
                                            .with_style(
                                                theme.workspace.external_location_message.container,
                                            )
                                            .boxed()
                                        },
                                    )
                                    .with_cursor_style(CursorStyle::PointingHand)
                                    .on_click(MouseButton::Left, move |_, cx| {
                                        cx.dispatch_action(JoinProject {
                                            project_id: leader_project_id,
                                            follow_user_id: leader_user_id,
                                        })
                                    })
                                    .aligned()
                                    .bottom()
                                    .right()
                                    .boxed(),
                                )
                            }
                        }
                        ParticipantLocation::UnsharedProject => Some(
                            Label::new(
                                format!(
                                    "{} is viewing an unshared Zed project",
                                    leader.user.github_login
                                ),
                                theme.workspace.external_location_message.text.clone(),
                            )
                            .contained()
                            .with_style(theme.workspace.external_location_message.container)
                            .aligned()
                            .bottom()
                            .right()
                            .boxed(),
                        ),
                        ParticipantLocation::External => Some(
                            Label::new(
                                format!(
                                    "{} is viewing a window outside of Zed",
                                    leader.user.github_login
                                ),
                                theme.workspace.external_location_message.text.clone(),
                            )
                            .contained()
                            .with_style(theme.workspace.external_location_message.container)
                            .aligned()
                            .bottom()
                            .right()
                            .boxed(),
                        ),
                    }
                } else {
                    None
                };

                Stack::new()
                    .with_child(
                        ChildView::new(pane, cx)
                            .contained()
                            .with_border(border)
                            .boxed(),
                    )
                    .with_children(prompt)
                    .boxed()
            }
            Member::Axis(axis) => axis.render(project, theme, follower_states, active_call, cx),
        }
    }

    fn collect_panes<'a>(&'a self, panes: &mut Vec<&'a ViewHandle<Pane>>) {
        match self {
            Member::Axis(axis) => {
                for member in &axis.members {
                    member.collect_panes(panes);
                }
            }
            Member::Pane(pane) => panes.push(pane),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PaneAxis {
    axis: Axis,
    members: Vec<Member>,
}

impl PaneAxis {
    fn split(
        &mut self,
        old_pane: &ViewHandle<Pane>,
        new_pane: &ViewHandle<Pane>,
        direction: SplitDirection,
    ) -> Result<()> {
        for (mut idx, member) in self.members.iter_mut().enumerate() {
            match member {
                Member::Axis(axis) => {
                    if axis.split(old_pane, new_pane, direction).is_ok() {
                        return Ok(());
                    }
                }
                Member::Pane(pane) => {
                    if pane == old_pane {
                        if direction.axis() == self.axis {
                            if direction.increasing() {
                                idx += 1;
                            }

                            self.members.insert(idx, Member::Pane(new_pane.clone()));
                        } else {
                            *member =
                                Member::new_axis(old_pane.clone(), new_pane.clone(), direction);
                        }
                        return Ok(());
                    }
                }
            }
        }
        Err(anyhow!("Pane not found"))
    }

    fn remove(&mut self, pane_to_remove: &ViewHandle<Pane>) -> Result<Option<Member>> {
        let mut found_pane_index = None;
        let mut remove_member = false;
        for (index, member) in self.members.iter_mut().enumerate() {
            match member {
                Member::Axis(axis) => {
                    if let Ok(last_pane) = axis.remove(pane_to_remove) {
                        if let Some(last_pane) = last_pane {
                            *member = last_pane;
                        }
                        found_pane_index = Some(index);
                        break;
                    }
                }

                Member::Pane(pane) => {
                    if pane == pane_to_remove {
                        found_pane_index = Some(index);
                        remove_member = true;
                        break;
                    }
                }
            }
        }

        if let Some(found_pane_index) = found_pane_index {
            if remove_member {
                self.members.remove(found_pane_index);
            } else if matches!(&self.members[found_pane_index], Member::Axis(axis) if axis.axis == self.axis)
            {
                let child = self.members.remove(found_pane_index);
                let members_to_splice = match child {
                    Member::Axis(axis) => axis.members,
                    _ => unreachable!(),
                };
                self.members
                    .splice(found_pane_index..found_pane_index, members_to_splice);
            }

            if self.members.len() == 1 {
                Ok(self.members.pop())
            } else {
                Ok(None)
            }
        } else {
            Err(anyhow!("Pane not found"))
        }
    }

    fn render(
        &self,
        project: &ModelHandle<Project>,
        theme: &Theme,
        follower_state: &FollowerStatesByLeader,
        active_call: Option<&ModelHandle<ActiveCall>>,
        cx: &mut RenderContext<Workspace>,
    ) -> ElementBox {
        let last_member_ix = self.members.len() - 1;
        Flex::new(self.axis)
            .with_children(self.members.iter().enumerate().map(|(ix, member)| {
                let mut member = member.render(project, theme, follower_state, active_call, cx);
                if ix < last_member_ix {
                    let mut border = theme.workspace.pane_divider;
                    border.left = false;
                    border.right = false;
                    border.top = false;
                    border.bottom = false;
                    match self.axis {
                        Axis::Vertical => border.bottom = true,
                        Axis::Horizontal => border.right = true,
                    }
                    member = Container::new(member).with_border(border).boxed();
                }

                FlexItem::new(member).flex(1.0, true).boxed()
            }))
            .boxed()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
pub enum SplitDirection {
    Up,
    Down,
    Left,
    Right,
}

impl SplitDirection {
    pub fn all() -> [Self; 4] {
        [Self::Up, Self::Down, Self::Left, Self::Right]
    }

    pub fn edge(&self, rect: RectF) -> f32 {
        match self {
            Self::Up => rect.min_y(),
            Self::Down => rect.max_y(),
            Self::Left => rect.min_x(),
            Self::Right => rect.max_x(),
        }
    }

    // Returns a new rectangle which shares an edge in SplitDirection and has `size` along SplitDirection
    pub fn along_edge(&self, rect: RectF, size: f32) -> RectF {
        match self {
            Self::Up => RectF::new(rect.origin(), Vector2F::new(rect.width(), size)),
            Self::Down => RectF::new(
                rect.lower_left() - Vector2F::new(0., size),
                Vector2F::new(rect.width(), size),
            ),
            Self::Left => RectF::new(rect.origin(), Vector2F::new(size, rect.height())),
            Self::Right => RectF::new(
                rect.upper_right() - Vector2F::new(size, 0.),
                Vector2F::new(size, rect.height()),
            ),
        }
    }

    pub fn axis(&self) -> Axis {
        match self {
            Self::Up | Self::Down => Axis::Vertical,
            Self::Left | Self::Right => Axis::Horizontal,
        }
    }

    pub fn increasing(&self) -> bool {
        match self {
            Self::Left | Self::Up => false,
            Self::Down | Self::Right => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use fs::FakeFs;
    use gpui::{Axis, TestAppContext, ViewContext};
    use project::Project;
    use settings::Settings;

    use crate::{pane, tests::TestItem, ItemHandle, Pane, SplitDirection, Workspace};

    use super::Member;

    pub fn default_item_factory(
        _workspace: &mut Workspace,
        cx: &mut ViewContext<Workspace>,
    ) -> Box<dyn ItemHandle> {
        Box::new(cx.add_view(|_| TestItem::new()))
    }

    #[gpui::test]
    async fn test_axis_closing_simplifies_split_tree(cx: &mut TestAppContext) {
        Settings::test_async(cx);
        let fs = FakeFs::new(cx.background());

        cx.update(|cx| pane::init(cx));

        let project = Project::test(fs, [], cx).await;
        let (_, workspace) = cx.add_window(|cx| Workspace::new(project, default_item_factory, cx));

        // Add an item to start
        workspace.update(cx, |workspace, cx| {
            let item = cx.add_view(|_| TestItem::new());
            let pane = workspace.active_pane().clone();
            Pane::add_item(workspace, &pane, Box::new(item), true, true, None, cx);
        });

        // Split right
        workspace.update(cx, |workspace, cx| {
            workspace.active_pane().update(cx, |pane, cx| {
                pane.split(SplitDirection::Right, cx);
            });
        });
        let right_pane = workspace.read_with(cx, |workspace, _| workspace.active_pane().clone());

        // Split down
        workspace.update(cx, |workspace, cx| {
            workspace
                .active_pane()
                .update(cx, |pane, cx| {
                    pane.split(SplitDirection::Down, cx);
                    workspace.active_pane()
                })
                .clone()
        });

        // Split right again
        workspace.update(cx, |workspace, cx| {
            workspace
                .active_pane()
                .update(cx, |pane, cx| {
                    pane.split(SplitDirection::Right, cx);
                    workspace.active_pane()
                })
                .clone()
        });

        // Activate the right_pane and close its item
        // leaving behind a vertical axis with a single member
        let right_pane_item_id =
            right_pane.read_with(cx, |pane, _| pane.active_item().unwrap().id());
        workspace
            .update(cx, |workspace, cx| {
                Pane::close_item(workspace, right_pane.clone(), right_pane_item_id, cx)
            })
            .await
            .unwrap();

        workspace.read_with(cx, |workspace, _| {
            let pane_group = workspace.center_pane_group();

            let pane_axis = match &pane_group.root {
                Member::Axis(axis) => axis,
                _ => panic!("Root group was not an axis"),
            };

            assert_eq!(Axis::Horizontal, pane_axis.axis);
            assert_eq!(3, pane_axis.members.len());
            assert!(pane_axis
                .members
                .iter()
                .all(|member| matches!(member, Member::Pane(_))));
        })
    }
}
