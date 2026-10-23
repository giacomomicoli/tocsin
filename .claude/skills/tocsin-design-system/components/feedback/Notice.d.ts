import * as React from "react";
/**
 * Footer notice line, the "replace these folders?" confirm banner, and the three-diamond loader.
 * @startingPoint section="Feedback" subtitle="Notice, confirm banner, loader" viewport="700x220"
 */
export interface NoticeProps { error?: boolean; children?: React.ReactNode; onDismiss?: () => void }
export function Notice(props: NoticeProps): JSX.Element;
export interface ConfirmBannerProps { title: string; children?: React.ReactNode; actions?: React.ReactNode }
export function ConfirmBanner(props: ConfirmBannerProps): JSX.Element;
/** Lit index advances every 250ms (Message::Tick). */
export interface LoaderProps { lit?: number; count?: number }
export function Loader(props: LoaderProps): JSX.Element;
