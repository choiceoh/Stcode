use gpui::{App, IntoElement, ParentElement};
use ui::{List, ListBulletItem, prelude::*};
use workspace::AppLaunchMode;

/// Centralized definitions for Zed AI plans
pub struct PlanDefinitions;

impl PlanDefinitions {
    fn agent_name(cx: &App) -> &'static str {
        if AppLaunchMode::is_stcode(cx) {
            "Stcode agent"
        } else {
            "Zed agent"
        }
    }

    pub fn free_plan(&self, _cx: &App) -> impl IntoElement {
        List::new()
            .child(ListBulletItem::new("2,000 accepted edit predictions"))
            .child(ListBulletItem::new(
                "Unlimited prompts with your AI API keys",
            ))
            .child(ListBulletItem::new("Unlimited use of external agents"))
    }

    pub fn pro_trial(&self, cx: &App, period: bool) -> impl IntoElement {
        List::new()
            .child(ListBulletItem::new(format!(
                "$20 of tokens in {}",
                Self::agent_name(cx)
            )))
            .child(ListBulletItem::new("Unlimited edit predictions"))
            .when(period, |this| {
                this.child(ListBulletItem::new(
                    "Try it out for 14 days, no credit card required",
                ))
            })
    }

    pub fn pro_plan(&self, cx: &App) -> impl IntoElement {
        List::new()
            .child(ListBulletItem::new(format!(
                "$5 of tokens in {}",
                Self::agent_name(cx)
            )))
            .child(ListBulletItem::new("Usage-based billing beyond $5"))
            .child(ListBulletItem::new("Unlimited edit predictions"))
    }

    pub fn business_plan(&self) -> impl IntoElement {
        List::new()
            .child(ListBulletItem::new("Unlimited edit predictions"))
            .child(ListBulletItem::new("Usage-based billing"))
    }

    pub fn student_plan(&self, cx: &App) -> impl IntoElement {
        List::new()
            .child(ListBulletItem::new("Unlimited edit predictions"))
            .child(ListBulletItem::new(format!(
                "$10 of tokens in {}",
                Self::agent_name(cx)
            )))
            .child(ListBulletItem::new(
                "Optional credit packs for additional usage",
            ))
    }
}
